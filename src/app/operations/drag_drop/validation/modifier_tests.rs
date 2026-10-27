use super::{
    is_valid_drop_target_for_operation, resolve_drag_operation_for_paths, should_confirm_drag_move,
    DragDropOperation,
};
use std::path::PathBuf;

#[test]
fn modifiers_apply_to_the_entire_mixed_selection_and_alt_wins() {
    let paths = vec![
        PathBuf::from(r"C:\Source\file.txt"),
        PathBuf::from(r"C:\Source\Folder"),
    ];
    let destination = PathBuf::from(r"D:\Destination");
    for (ctrl, shift, alt, expected) in [
        (false, false, false, DragDropOperation::Move),
        (false, true, false, DragDropOperation::Move),
        (true, false, false, DragDropOperation::Copy),
        (true, true, false, DragDropOperation::Copy),
        (false, false, true, DragDropOperation::Link),
        (true, false, true, DragDropOperation::Link),
        (true, true, true, DragDropOperation::Link),
    ] {
        assert_eq!(
            resolve_drag_operation_for_paths(&paths, &destination, ctrl, shift, alt),
            expected
        );
    }
}

#[test]
fn same_folder_accepts_multi_copy_and_links_but_not_moves() {
    let paths = vec![
        PathBuf::from(r"C:\Source\file.txt"),
        PathBuf::from(r"C:\Source\Folder"),
    ];
    let destination = PathBuf::from(r"C:\Source");
    assert!(!is_valid_drop_target_for_operation(
        &paths,
        &destination,
        DragDropOperation::Move
    ));
    assert!(is_valid_drop_target_for_operation(
        &paths,
        &destination,
        DragDropOperation::Copy
    ));
    assert!(is_valid_drop_target_for_operation(
        &paths,
        &destination,
        DragDropOperation::Link
    ));
    assert!(!should_confirm_drag_move(DragDropOperation::Link));
}

#[test]
fn selection_from_multiple_parents_can_be_copied_to_one_of_them() {
    let paths = vec![
        PathBuf::from(r"C:\Source\file.txt"),
        PathBuf::from(r"C:\Other\file.txt"),
    ];
    let destination = PathBuf::from(r"C:\Source");
    assert!(is_valid_drop_target_for_operation(
        &paths,
        &destination,
        DragDropOperation::Copy
    ));
    assert!(!is_valid_drop_target_for_operation(
        &paths,
        &destination,
        DragDropOperation::Move
    ));
}

#[test]
fn copying_selection_into_itself_or_a_descendant_is_rejected() {
    let paths = vec![
        PathBuf::from(r"C:\Source\Folder"),
        PathBuf::from(r"C:\Source\file.txt"),
    ];
    for destination in [r"C:\Source\Folder", r"C:\Source\Folder\Child"] {
        assert!(!is_valid_drop_target_for_operation(
            &paths,
            &PathBuf::from(destination),
            DragDropOperation::Copy
        ));
    }
}

#[test]
fn archive_entries_keep_copy_semantics_even_with_alt_and_ctrl() {
    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("archive.zip");
    std::fs::write(&archive, b"placeholder").unwrap();
    let paths = vec![archive.join("file.txt")];
    assert_eq!(
        resolve_drag_operation_for_paths(&paths, temp.path(), true, false, true),
        DragDropOperation::Copy
    );
}
