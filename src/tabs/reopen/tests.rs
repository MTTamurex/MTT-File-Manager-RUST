use super::super::*;

#[test]
fn empty_closed_history_does_not_change_tabs() {
    let mut manager = TabManager::new();
    let id = manager.active().id;
    assert!(!manager.can_reopen_closed_tab());
    assert!(!manager.reopen_closed_tab());
    assert_eq!(manager.count(), 1);
    assert_eq!(manager.active().id, id);
}

#[test]
fn reopens_in_reverse_close_order_with_new_ids() {
    let mut manager = TabManager::new();
    for path in [r"C:\FolderA", r"C:\FolderB", r"C:\FolderC"] {
        assert!(manager.new_tab_at(path));
    }
    let old_ids: Vec<_> = manager.tabs.iter().map(|tab| tab.id).collect();
    manager.close_tab(1);
    manager.close_tab(2);
    for path in [r"C:\FolderC", r"C:\FolderA"] {
        assert!(manager.can_reopen_closed_tab());
        assert!(manager.reopen_closed_tab());
        assert_eq!(manager.active().path, path);
        assert!(!old_ids.contains(&manager.active().id));
        assert!(!manager.active().pinned);
    }
    assert!(!manager.has_closed_tabs());
}

#[test]
fn retains_only_the_ten_most_recently_closed_tabs() {
    let mut manager = TabManager::new();
    for index in 0..12 {
        assert!(manager.new_tab_at(&format!(r"C:\Folder{index}")));
        manager.close_active_tab();
    }
    for index in (2..12).rev() {
        assert!(manager.reopen_closed_tab());
        assert_eq!(manager.active().path, format!(r"C:\Folder{index}"));
    }
    assert!(!manager.reopen_closed_tab());
}

#[test]
fn tab_limit_keeps_the_closed_snapshot_available() {
    let mut manager = TabManager::new();
    assert!(manager.new_tab_at(r"C:\Folder"));
    manager.close_active_tab();
    while manager.can_add_tab() {
        assert!(manager.new_tab());
    }
    let active_id = manager.active().id;
    assert!(!manager.can_reopen_closed_tab());
    assert!(!manager.reopen_closed_tab());
    assert_eq!(manager.count(), MAX_TABS);
    assert_eq!(manager.active().id, active_id);
    assert!(manager.has_closed_tabs());
    assert_eq!(manager.closed_tabs.len(), 1);
    assert_eq!(manager.closed_tabs[0].path, r"C:\Folder");

    manager.close_active_tab();
    assert!(manager.reopen_closed_tab());
    assert_eq!(manager.closed_tabs.len(), 1);
    assert_eq!(manager.closed_tabs[0].path, r"C:\Folder");
}

#[test]
fn restores_retained_state_without_stale_listing_or_rebuilds() {
    let mut manager = TabManager::new();
    assert!(manager.new_tab_at(r"C:\FolderA"));
    let tab = manager.active_mut();
    tab.navigate_to(r"C:\FolderB");
    tab.title = "Folder B".into();
    tab.view_mode = ViewMode::List;
    tab.sort_mode = SortMode::Size;
    tab.sort_descending = true;
    tab.search_query = "file".into();
    tab.show_preview_panel = false;
    tab.pending_items_rebuild = true;
    tab.pending_items_count = 5;
    tab.inactive_final_items_rebuild_pending = true;
    tab.is_loading_folder = true;
    tab.items = Arc::new(vec![FileEntry::from_path(
        PathBuf::from(r"C:\FolderB\file.txt"),
        false,
    )]);
    tab.all_items = tab.items.clone();
    manager.close_active_tab();

    assert!(manager.reopen_closed_tab());
    let tab = manager.active();
    assert_eq!(tab.path, r"C:\FolderB");
    assert_eq!(tab.path_input, r"C:\FolderB");
    assert_eq!(tab.title, "Folder B");
    assert!(tab.can_go_back());
    assert_eq!(tab.view_mode, ViewMode::List);
    assert_eq!(tab.sort_mode, SortMode::Size);
    assert!(tab.sort_descending);
    assert_eq!(tab.search_query, "file");
    assert!(!tab.show_preview_panel);
    assert!(tab.items.is_empty());
    assert!(tab.all_items.is_empty());
    assert!(!tab.is_loading_folder);
    assert!(!tab.pending_items_rebuild);
    assert_eq!(tab.pending_items_count, 0);
    assert!(!tab.inactive_final_items_rebuild_pending);
}

#[test]
fn reopens_special_locations_with_correct_view_flags() {
    for path in [COMPUTER_VIEW_ID, RECYCLE_BIN_VIEW_ID, "::tag::1"] {
        let mut manager = TabManager::new();
        assert!(manager.new_tab_at(path));
        manager.close_active_tab();
        assert!(manager.reopen_closed_tab());
        assert_eq!(manager.active().path, path);
        assert_eq!(manager.active().is_computer_view, path == COMPUTER_VIEW_ID);
        assert_eq!(
            manager.active().is_recycle_bin_view,
            path == RECYCLE_BIN_VIEW_ID
        );
    }
}
