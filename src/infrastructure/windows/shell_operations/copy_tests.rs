use super::super::shfile_ops::{copy_items_with_shell, copy_operation_flags};
use super::{copy_item_with_file_op, copy_items_with_file_op};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::FOF_RENAMEONCOLLISION;

#[test]
fn same_folder_copy_renames_single_and_multiple_items_without_touching_originals() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("file.txt");
    let folder = temp.path().join("Folder");
    std::fs::write(&file, b"original").unwrap();
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("child.txt"), b"child").unwrap();

    assert!(copy_item_with_file_op(&file, temp.path(), HWND::default()));
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 3);
    assert!(copy_items_with_file_op(
        &[file.clone(), folder.clone()],
        temp.path(),
        HWND::default()
    ));
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 5);
    assert_eq!(std::fs::read(&file).unwrap(), b"original");
    let directories: Vec<_> = std::fs::read_dir(temp.path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .collect();
    assert_eq!(directories.len(), 2);
    for directory in directories {
        assert_eq!(
            std::fs::read(directory.join("child.txt")).unwrap(),
            b"child"
        );
    }
}

#[test]
fn same_folder_legacy_fallback_copies_entire_selection() {
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("file.txt");
    let second = temp.path().join("other.txt");
    std::fs::write(&first, b"first").unwrap();
    std::fs::write(&second, b"second").unwrap();
    assert!(copy_items_with_shell(
        &[first.clone(), second.clone()],
        temp.path(),
        HWND::default()
    ));
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 4);
    assert_eq!(std::fs::read(&first).unwrap(), b"first");
    assert_eq!(std::fs::read(&second).unwrap(), b"second");
}

#[test]
fn different_folder_copy_preserves_existing_collision_policy() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("Source");
    let destination = temp.path().join("Destination");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(&destination).unwrap();
    let paths = vec![source.join("file.txt")];
    assert_eq!(
        copy_operation_flags(&paths, &destination).0 & FOF_RENAMEONCOLLISION.0,
        0
    );
    assert_ne!(
        copy_operation_flags(&paths, &source).0 & FOF_RENAMEONCOLLISION.0,
        0
    );
}
