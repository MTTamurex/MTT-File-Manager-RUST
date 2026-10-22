use super::*;

#[test]
fn folder_timestamps_are_independent_between_tabs() {
    let mut manager = TabManager::new_at_path(r"C:\Folder");
    let modified = Some((PathBuf::from(r"C:\Folder"), 100));
    let created = Some((PathBuf::from(r"C:\Folder"), 50));
    manager.active_mut().current_folder_modified_hint = modified.clone();
    manager.active_mut().current_folder_created_hint = created.clone();
    assert!(manager.new_tab_at(r"D:\Folder"));
    assert!(manager.active().current_folder_modified_hint.is_none());
    assert!(manager.active().current_folder_created_hint.is_none());
    manager.active_mut().current_folder_modified_hint = Some((PathBuf::from(r"D:\Folder"), 200));
    for index in [0, 1, 0] {
        manager.active_tab = index;
        let tab = manager.active();
        if index == 0 {
            assert_eq!(tab.current_folder_modified_hint, modified);
            assert_eq!(tab.current_folder_created_hint, created);
        } else {
            assert_eq!(tab.current_folder_modified_hint.as_ref().unwrap().1, 200);
            assert!(tab.current_folder_created_hint.is_none());
        }
    }
}

#[test]
fn changing_tab_path_clears_previous_folder_timestamps() {
    let mut tab = TabState::new_at_path(1, r"C:\Folder");
    for action in 0..4 {
        tab.current_folder_modified_hint = Some((PathBuf::from(&tab.path), 100));
        tab.current_folder_created_hint = Some((PathBuf::from(&tab.path), 50));
        match action {
            0 => tab.navigate_to(r"D:\Folder"),
            1 => assert!(tab.go_back()),
            2 => assert!(tab.go_forward()),
            _ => tab.redirect_to_history_current(),
        }
        assert!(tab.current_folder_modified_hint.is_none());
        assert!(tab.current_folder_created_hint.is_none());
    }
}

#[test]
fn sidebar_network_locations_collapse_is_independent_between_tabs() {
    let mut first = TabState::new_at_computer(1);
    let second = TabState::new_at_path(2, r"C:\Folder");
    assert!(!first.collapse_network_locations);
    assert!(!second.collapse_network_locations);
    first.collapse_network_locations = true;
    assert!(!second.collapse_network_locations);
    assert!(!first.collapse_network_drives);
    assert!(first.clone().collapse_network_locations);
    assert!(
        !first
            .into_lightweight_closed_snapshot()
            .collapse_network_locations
    );
}

#[test]
fn tabs_created_by_path_recognize_special_locations() {
    for (path, computer, recycle_bin) in [
        (COMPUTER_VIEW_ID, true, false),
        (RECYCLE_BIN_VIEW_ID, false, true),
        (r"C:\Folder", false, false),
        (r"C:\archive.zip", false, false),
    ] {
        let tab = TabState::new_at_path(1, path);
        assert_eq!(tab.is_computer_view, computer);
        assert_eq!(tab.is_recycle_bin_view, recycle_bin);
        assert_eq!(tab.path, path);

        let mut manager = TabManager::new_at_path(r"C:\Source");
        assert!(manager.new_tab_at(path));
        assert_eq!(manager.active().path, path);
        assert_eq!(manager.active().is_computer_view, computer);
        assert_eq!(manager.active().is_recycle_bin_view, recycle_bin);
        assert_eq!(manager.tabs[0].path, r"C:\Source");
        assert_eq!(manager.count(), 2);
    }
}
