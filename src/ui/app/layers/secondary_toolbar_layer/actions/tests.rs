use super::delete_targets;
use std::path::PathBuf;

#[test]
fn delete_uses_selected_path_without_a_list_index() {
    let selected = PathBuf::from(r"C:\Folder\file.txt");

    assert_eq!(
        delete_targets(Some(&selected), std::iter::empty()),
        vec![selected],
    );
}

#[test]
fn delete_uses_all_multi_selected_paths_instead_of_single_selection() {
    let selected = PathBuf::from(r"C:\Folder\other.txt");
    let multiple = vec![
        PathBuf::from(r"C:\Folder\file.txt"),
        PathBuf::from(r"C:\Folder\another.txt"),
    ];

    assert_eq!(delete_targets(Some(&selected), multiple.iter()), multiple);
}

#[test]
fn delete_accepts_multi_selection_without_a_single_selection() {
    let multiple = vec![PathBuf::from(r"C:\Folder\file.txt")];

    assert_eq!(delete_targets(None, multiple.iter()), multiple);
}

#[test]
fn delete_without_selection_has_no_targets() {
    assert!(delete_targets(None, std::iter::empty()).is_empty());
}
