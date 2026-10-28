use super::*;
use crate::{infrastructure::app_state_db::AppStateDb, tabs::TabManager};

fn saved() -> String {
    let db = AppStateDb::new_in_memory().unwrap();
    let sessions = [r"C:\Example", r"D:\Other", r"E:\Example"]
        .map(|path| TabManagerSession::from_tab_manager(&TabManager::new_at_path(path)));
    assert!(record::save(&db, sessions.into()));
    db.get_preference(record::KEY).unwrap()
}

#[test]
fn cold_restore_assigns_exactly_one_session_to_each_additional_instance() {
    let saved = saved();
    let (first, remaining) =
        restoration_plan(Some(&saved), Some("previous_session"), true).unwrap();
    assert_eq!(first.active_path(), r"C:\Example");
    assert_eq!(
        remaining
            .iter()
            .map(TabManagerSession::active_path)
            .collect::<Vec<_>>(),
        [r"D:\Other", r"E:\Example"]
    );
    let startup = Startup {
        first,
        remaining: Mutex::new(Some(remaining)),
        acknowledgement: Mutex::new(None),
    };
    assert_eq!(startup.remaining.lock().unwrap().take().unwrap().len(), 2);
    assert!(startup.remaining.lock().unwrap().take().is_none());
}

#[test]
fn fixed_startup_and_warm_or_single_instance_launches_do_not_reopen_the_entire_set() {
    let saved = saved();
    assert!(restoration_plan(Some(&saved), Some("computer"), true).is_none());
    assert!(restoration_plan(Some(&saved), Some("custom"), true).is_none());
    let (first, remaining) = restoration_plan(Some(&saved), None, false).unwrap();
    assert_eq!(first.active_path(), r"C:\Example");
    assert!(remaining.is_empty());
    assert_eq!(record::decode(&saved).unwrap().len(), 3);
    assert!(restoration_plan(Some("invalid"), None, true).is_none());
}

#[test]
fn invalid_restoration_tokens_cannot_fall_through_to_normal_startup() {
    assert!(receive_startup(0, "invalid").is_err());
    assert!(receive_startup(0, "").is_err());
}
