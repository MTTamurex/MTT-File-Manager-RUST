use super::*;
use crate::tabs::TabManager;

fn session(path: &str) -> TabManagerSession {
    let mut manager = TabManager::new_at_path(path);
    manager.new_tab_at(r"D:\Other");
    manager.tabs[0].pinned = true;
    TabManagerSession::from_tab_manager(&manager)
}

#[test]
fn last_individual_close_replaces_the_previous_restart_set() {
    let db = AppStateDb::new_in_memory().unwrap();
    let first = session(r"C:\Example");
    let last = session(r"E:\Example");
    assert!(save(&db, vec![first.clone(), last.clone()]));
    assert!(save(&db, vec![first]));
    assert!(save(&db, vec![last.clone()]));
    assert_eq!(decode(&db.get_preference(KEY).unwrap()).unwrap(), [last]);
}

#[test]
fn an_earlier_close_finishing_late_cannot_replace_the_last_closed_instance() {
    let db = AppStateDb::new_in_memory().unwrap();
    let earlier = exit_order();
    let later = exit_order();
    assert!(later > earlier);
    let last = session(r"E:\Example");
    assert_eq!(save_at(&db, vec![last.clone()], later), SaveOutcome::Saved);
    assert_eq!(
        save_at(&db, vec![session(r"C:\Example")], earlier),
        SaveOutcome::Superseded
    );
    assert_eq!(decode(&db.get_preference(KEY).unwrap()).unwrap(), [last]);
}

#[test]
fn an_old_individual_close_cannot_overwrite_a_newer_global_exit() {
    let db = AppStateDb::new_in_memory().unwrap();
    let old_close = exit_order();
    let group = vec![session(r"C:\Example"), session(r"E:\Example")];
    assert!(save(&db, group.clone()));
    assert_eq!(
        save_at(&db, vec![session(r"D:\Other")], old_close),
        SaveOutcome::Superseded
    );
    assert_eq!(decode(&db.get_preference(KEY).unwrap()).unwrap(), group);
}

#[test]
fn late_background_preferences_cannot_overwrite_a_saved_global_exit() {
    let db = AppStateDb::new_in_memory().unwrap();
    let sessions = vec![session(r"C:\Example"), session(r"E:\Example")];
    assert!(save(&db, sessions.clone()));
    db.set_preferences_batch(&[
        ("tab_session", sessions[1].to_json().unwrap()),
        ("dual_panel_enabled", "false".into()),
    ])
    .unwrap();
    assert_eq!(decode(&db.get_preference(KEY).unwrap()).unwrap(), sessions);
}

#[test]
fn a_delayed_concurrent_group_save_reports_superseded_without_replacing_the_newer_record() {
    let db = std::sync::Arc::new(AppStateDb::new_in_memory().unwrap());
    let earlier = exit_order();
    let later = exit_order();
    let (release, wait) = std::sync::mpsc::channel();
    let worker_db = db.clone();
    let worker = std::thread::spawn(move || {
        wait.recv().unwrap();
        save_at(
            &worker_db,
            vec![session(r"C:\Example"), session(r"D:\Other")],
            earlier,
        )
    });
    let newest = session(r"E:\Example");
    assert_eq!(
        save_at(&db, vec![newest.clone()], later),
        SaveOutcome::Saved
    );
    release.send(()).unwrap();
    assert_eq!(worker.join().unwrap(), SaveOutcome::Superseded);
    assert_eq!(decode(&db.get_preference(KEY).unwrap()).unwrap(), [newest]);
}

#[test]
fn records_without_close_order_remain_readable_and_replaceable() {
    let db = AppStateDb::new_in_memory().unwrap();
    let old = session(r"C:\Example");
    let legacy = serde_json::json!({"version": 1, "sessions": [old.clone()]}).to_string();
    db.set_preference(KEY, &legacy).unwrap();
    assert_eq!(decode(&legacy).unwrap(), [old]);
    let newest = session(r"D:\Other");
    assert!(save(&db, vec![newest.clone()]));
    assert_eq!(decode(&db.get_preference(KEY).unwrap()).unwrap(), [newest]);
}

#[test]
fn restart_set_keeps_pins_active_workspaces_and_both_panel_selections() {
    let mut value = serde_json::to_value(session(r"C:\Example")).unwrap();
    let panel = |id, path: &str| {
        serde_json::json!({
            "pinned": true, "id": id, "path": path, "view_mode": "grid", "sort_mode": "name",
            "sort_descending": false, "folders_position": "first", "group_mode": "none", "group_descending": false,
            "active_tag_filter": null, "column_widths": { "values": vec![120.0; 12] }
        })
    };
    value["tabs"][1]["dual_panel_enabled"] = true.into();
    value["tabs"][1]["dual_panel_active"] = "right".into();
    value["tabs"][1]["dual_panel_tabs"] = serde_json::json!({
        "left": { "active_tab": 1, "tabs": [panel(1, r"C:\Example"), panel(2, r"D:\Other")] },
        "right": { "active_tab": 0, "tabs": [panel(3, r"E:\Example")] }
    });
    let split = TabManagerSession::from_json(&value.to_string()).unwrap();
    let db = AppStateDb::new_in_memory().unwrap();
    assert!(save(&db, vec![split.clone(), session(r"Z:\Example")]));
    let restored = decode(&db.get_preference(KEY).unwrap()).unwrap();
    assert_eq!(restored[0], split);
    assert_eq!(restored[0].active_tab, 1);
    assert!(restored[0].tabs[0].pinned);
    assert_eq!(restored[0].tabs[1].dual_panel_active, "right");
    assert_eq!(
        restored[0].tabs[1]
            .dual_panel_tabs
            .as_ref()
            .unwrap()
            .left
            .active_tab,
        1
    );
    assert!(
        restored[0].tabs[1]
            .dual_panel_tabs
            .as_ref()
            .unwrap()
            .right
            .tabs[0]
            .pinned
    );
}

#[test]
fn invalid_or_empty_records_never_replace_the_previous_valid_session() {
    let db = AppStateDb::new_in_memory().unwrap();
    assert!(save(&db, vec![session(r"C:\Example")]));
    let previous = db.get_preference(KEY).unwrap();
    assert!(!save(&db, Vec::new()));
    let mut invalid = session(r"D:\Other");
    invalid.tabs.clear();
    assert!(!save(&db, vec![invalid]));
    assert_eq!(db.get_preference(KEY).unwrap(), previous);
    assert!(decode(r#"{"version":99,"sessions":[]}"#).is_none());
    assert!(decode(&"x".repeat(MAX_BYTES + 1)).is_none());
}
