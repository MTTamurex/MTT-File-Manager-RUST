use super::*;

#[test]
fn empty_selection_has_no_fallback_to_the_current_folder() {
    assert!(selection_paths(None, std::iter::empty()).is_empty());
    assert!(!supported_targets(&[]));
}

#[test]
fn focused_file_is_used_only_without_a_multi_selection() {
    let focused = PathBuf::from(r"C:\Folder\focused.txt");
    assert_eq!(
        selection_paths(Some(&focused), std::iter::empty()),
        vec![focused.clone()]
    );
    let selected = [
        PathBuf::from(r"C:\Folder\first.txt"),
        PathBuf::from(r"C:\Folder\second.txt"),
    ];
    assert_eq!(selection_paths(Some(&focused), selected.iter()), selected);
}

#[test]
fn independent_panel_selections_do_not_share_targets() {
    let left = [PathBuf::from(r"C:\Left\file.txt")];
    let right = [PathBuf::from(r"C:\Right\file.txt")];
    assert_eq!(selection_paths(None, left.iter()), left);
    assert_eq!(selection_paths(None, right.iter()), right);
}

#[test]
fn drives_network_roots_and_virtual_views_are_never_file_targets() {
    for path in [
        "",
        "C:",
        r"C:\",
        r"\\?\C:",
        r"\\?\C:\",
        r"\\server",
        r"\\server\share",
        r"\\?\UNC\server\share",
        "::computer::",
        "::recycle_bin::",
        "::tag::1",
    ] {
        assert!(!supported_targets(&[PathBuf::from(path)]), "{path}");
    }
    let file = PathBuf::from(r"C:\Folder\file.txt");
    assert!(!supported_targets(&[file, PathBuf::from(r"C:\")]));
    assert!(supported_targets(&[
        PathBuf::from(r"C:\Folder\file.txt"),
        PathBuf::from(r"\\server\share\folder\file.txt")
    ]));
}
