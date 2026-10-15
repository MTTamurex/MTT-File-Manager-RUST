use super::*;

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
