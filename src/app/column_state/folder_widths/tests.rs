use super::*;

fn location(path: &str) -> WidthLocation {
    WidthLocation::for_view(path, false, false).unwrap()
}

#[test]
fn edits_are_shared_by_location_and_survive_restart_without_inheritance() {
    let db = Arc::new(AppStateDb::new_in_memory().unwrap());
    let ctx = egui::Context::default();
    let a = location(r"C:\Example\Folder");
    let parent = location(r"C:\Example");
    let defaults = [(ColumnId::Name, 300.0), (ColumnId::Size, 160.0)];
    let mut state = FolderWidths::default();
    state.start(db.clone());
    state.edit(&a, &[(ColumnId::Name, 500.0)], &ctx);
    assert_eq!(state.frame(&a, &defaults, &ctx)[0].1, 500.0);
    assert_eq!(state.frame(&parent, &defaults, &ctx)[0].1, 300.0);
    assert!(state.flush(Duration::from_secs(2)));
    let mut restored = FolderWidths::default();
    restored.start(db);
    restored.frame(&a, &defaults, &ctx);
    let response = restored
        .worker
        .as_ref()
        .unwrap()
        .results
        .recv_timeout(Duration::from_secs(2))
        .unwrap();
    restored.handle(response);
    assert_eq!(restored.frame(&a, &defaults, &ctx)[0].1, 500.0);
}

#[test]
fn late_database_load_cannot_overwrite_a_resize_done_while_loading() {
    let ctx = egui::Context::default();
    let a = location(r"C:\Example");
    let mut state = FolderWidths::default();
    state.edit(&a, &[(ColumnId::Name, 550.0)], &ctx);
    state.handle(Response::Loaded(
        a.clone(),
        Ok(vec![(ColumnId::Name, 200.0), (ColumnId::Size, 220.0)]),
    ));
    let widths = state.frame(
        &a,
        &[(ColumnId::Name, 300.0), (ColumnId::Size, 160.0)],
        &ctx,
    );
    assert_eq!(widths, [(ColumnId::Name, 550.0), (ColumnId::Size, 220.0)]);
}

#[test]
fn blocked_database_does_not_block_frame_edit_or_final_save_deadline() {
    let db = Arc::new(AppStateDb::new_in_memory().unwrap());
    let mut state = FolderWidths::default();
    state.start(db.clone());
    let guard = db.test_hold_writer();
    let ctx = egui::Context::default();
    let a = location(r"C:\Example");
    let start = Instant::now();
    state.frame(&a, &[(ColumnId::Name, 300.0)], &ctx);
    for width in 400..500 {
        state.edit(&a, &[(ColumnId::Name, width as f32)], &ctx);
    }
    assert_eq!(state.dirty.len(), 1);
    assert!(!state.flush(Duration::from_millis(20)));
    assert!(start.elapsed() < Duration::from_secs(1));
    drop(guard);
    assert!(state.flush(Duration::from_secs(2)));
    assert_eq!(
        db.load_column_widths(&a).unwrap(),
        [(ColumnId::Name, 499.0)]
    );
}

#[test]
fn acknowledging_an_older_write_preserves_newer_edits() {
    let mut state = FolderWidths::default();
    let ctx = egui::Context::default();
    let a = location(r"C:\Example");
    state.edit(&a, &[(ColumnId::Name, 400.0)], &ctx);
    state.in_flight = Some(state.dirty.values().cloned().collect());
    state.edit(&a, &[(ColumnId::Name, 500.0)], &ctx);
    state.handle(Response::Saved(true));
    assert_eq!(state.dirty.len(), 1);
    assert_eq!(state.dirty.values().next().unwrap().width, 500.0);
}

#[test]
fn failed_saves_and_cache_eviction_preserve_pending_edits() {
    let mut state = FolderWidths::default();
    let ctx = egui::Context::default();
    let a = location(r"C:\Example");
    state.edit(&a, &[(ColumnId::Name, 450.0)], &ctx);
    state.in_flight = Some(state.dirty.values().cloned().collect());
    assert!(!state.handle(Response::Saved(false)));
    assert_eq!(state.dirty.len(), 1);
    assert!(state.save_at.unwrap() > Instant::now());
    for index in 0..140 {
        state.entry(&WidthLocation(format!("location:folder_{index}")));
    }
    assert!(!state.entries.contains(&a));
    assert_eq!(state.entry(&a).values[&ColumnId::Name], 450.0);
    state.handle(Response::Loaded(
        a.clone(),
        Ok(vec![(ColumnId::Name, 200.0)]),
    ));
    assert_eq!(state.entry(&a).values[&ColumnId::Name], 450.0);
}

#[test]
fn special_views_persist_independent_profiles_and_visits_do_not_create_records() {
    let db = Arc::new(AppStateDb::new_in_memory().unwrap());
    let mut state = FolderWidths::default();
    state.start(db.clone());
    let ctx = egui::Context::default();
    let computer = WidthLocation::for_view("", true, false).unwrap();
    let recycle = WidthLocation::for_view("", false, true).unwrap();
    let folder = location(r"C:\Example");
    state.frame(&folder, &[(ColumnId::Name, 300.0)], &ctx);
    state.edit(&computer, &[(ColumnId::Name, 450.0)], &ctx);
    state.edit(&recycle, &[(ColumnId::Name, 550.0)], &ctx);
    assert!(state.flush(Duration::from_secs(2)));
    assert_eq!(
        db.load_column_widths(&computer).unwrap(),
        [(ColumnId::Name, 450.0)]
    );
    assert_eq!(
        db.load_column_widths(&recycle).unwrap(),
        [(ColumnId::Name, 550.0)]
    );
    assert!(db.load_column_widths(&folder).unwrap().is_empty());
}
