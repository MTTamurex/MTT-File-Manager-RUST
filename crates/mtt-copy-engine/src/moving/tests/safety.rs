use super::*;
use std::os::windows::fs::OpenOptionsExt;
use windows::Win32::Storage::FileSystem::{FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE};

#[test]
fn rename_preserves_exact_names_at_every_buffer_alignment_and_for_unicode() {
    let (root, source, target) = fixture();
    let mut names: Vec<_> = (1..=16)
        .map(|length| format!("{}.txt", "a".repeat(length)))
        .collect();
    names.push("文件-é.txt".to_owned());
    for name in &names {
        fs::write(source.join(name), name.as_bytes()).unwrap();
    }
    let paths = names.iter().map(|name| source.join(name)).collect();
    let (result, _) = run(paths, target.clone(), |_| panic!("No conflicts"));
    let summary = result.unwrap();
    assert_eq!(summary.failures, 0, "{:?}", summary.first_error);
    assert_eq!(summary.files_moved, names.len());
    assert_eq!(fs::read_dir(&target).unwrap().count(), names.len());
    for name in &names {
        assert!(!source.join(name).exists());
        assert_eq!(fs::read(target.join(name)).unwrap(), name.as_bytes());
        assert!(summary
            .moved_pairs
            .contains(&(source.join(name), target.join(name))));
    }
    root.close().unwrap();
}

#[test]
fn readonly_cross_volume_source_is_retained_after_its_copy_is_published() {
    let (root, source, target) = fixture();
    let file = source.join("file.txt");
    fs::write(&file, b"contents").unwrap();
    let mut permissions = fs::metadata(&file).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&file, permissions).unwrap();
    let progress = Progress::default();
    let plan = one_file_plan(&file, &target, &progress);
    let retained = transfer_with(
        &plan.files[0],
        &progress,
        None,
        CacheMode::Buffered,
        true,
        || Ok(()),
    )
    .unwrap();
    assert_eq!(retained.unwrap().kind(), io::ErrorKind::PermissionDenied);
    for path in [&file, &target.join("file.txt")] {
        assert_eq!(fs::read(path).unwrap(), b"contents");
        let mut permissions = fs::metadata(path).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        fs::set_permissions(path, permissions).unwrap();
    }
    root.close().unwrap();
}

#[test]
fn folder_keep_both_uses_the_final_name_and_folder_skip_preserves_both_trees() {
    for action in [ConflictAction::KeepBoth, ConflictAction::Skip] {
        let (root, source, target) = fixture();
        fs::write(source.join("new.txt"), b"new").unwrap();
        fs::create_dir(target.join("source")).unwrap();
        fs::write(target.join("source/old.txt"), b"old").unwrap();
        let (result, _) = run(vec![source.clone()], target.clone(), |_| action);
        let summary = result.unwrap();
        assert_eq!(summary.failures, 0, "{:?}", summary.first_error);
        assert_eq!(fs::read(target.join("source/old.txt")).unwrap(), b"old");
        if action == ConflictAction::KeepBoth {
            assert!(!source.exists());
            let moved = target.join("source (2)").join("new.txt");
            assert_eq!(fs::read(&moved).unwrap(), b"new");
            assert!(summary
                .moved_pairs
                .contains(&(source.clone(), target.join("source (2)"))));
        } else {
            assert_eq!(summary.items_skipped, 1);
            assert_eq!(fs::read(source.join("new.txt")).unwrap(), b"new");
            assert!(summary.moved_pairs.is_empty());
        }
        root.close().unwrap();
    }
}

#[test]
fn source_guard_blocks_writers_and_renames_during_a_cross_volume_copy() {
    let (root, source, target) = fixture();
    let file = source.join("file.txt");
    fs::write(&file, b"contents").unwrap();
    let progress = Progress::default();
    let plan = one_file_plan(&file, &target, &progress);
    let retained = transfer_with(
        &plan.files[0],
        &progress,
        None,
        CacheMode::Buffered,
        true,
        || {
            let writer = fs::OpenOptions::new()
                .write(true)
                .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0)
                .open(&file);
            assert_eq!(writer.unwrap_err().raw_os_error(), Some(32));
            assert!(fs::rename(&file, source.join("changed.txt")).is_err());
            assert!(fs::rename(&source, root.path().join("changed")).is_err());
            Ok(())
        },
    )
    .unwrap();
    assert!(retained.is_none(), "{retained:?}");
    assert!(!file.exists());
    root.close().unwrap();
}

#[test]
fn changed_published_destination_never_authorizes_removing_the_source() {
    let (root, source, target) = fixture();
    let file = source.join("file.txt");
    fs::write(&file, b"contents").unwrap();
    let progress = Progress::default();
    let plan = one_file_plan(&file, &target, &progress);
    let retained = transfer_with(
        &plan.files[0],
        &progress,
        None,
        CacheMode::Buffered,
        true,
        || {
            fs::write(target.join("file.txt"), b"changed externally")?;
            Ok(())
        },
    )
    .unwrap();
    assert!(retained.is_some());
    assert_eq!(fs::read(&file).unwrap(), b"contents");
    assert_eq!(
        fs::read(target.join("file.txt")).unwrap(),
        b"changed externally"
    );
    root.close().unwrap();
}

#[test]
fn late_source_directory_children_and_replaced_directories_are_not_removed() {
    for replace in [false, true] {
        let (root, source, target) = fixture();
        let directory = source::Directory::capture(&source, &target.join("source")).unwrap();
        if replace {
            fs::rename(&source, root.path().join("original")).unwrap();
            fs::create_dir(&source).unwrap();
        }
        fs::write(source.join("late.txt"), b"preserved").unwrap();
        assert!(directory.remove_empty().is_err());
        assert_eq!(fs::read(source.join("late.txt")).unwrap(), b"preserved");
        root.close().unwrap();
    }
}

#[test]
fn open_writer_blocks_both_rename_and_copy_without_any_publication() {
    for copy in [false, true] {
        let (root, source, target) = fixture();
        let file = source.join("file.txt");
        fs::write(&file, b"contents").unwrap();
        let progress = Progress::default();
        let plan = one_file_plan(&file, &target, &progress);
        let writer = fs::OpenOptions::new()
            .write(true)
            .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0)
            .open(&file)
            .unwrap();
        assert!(transfer_with(
            &plan.files[0],
            &progress,
            None,
            CacheMode::Buffered,
            copy,
            || Ok(())
        )
        .is_err());
        assert_eq!(fs::read(&file).unwrap(), b"contents");
        assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
        drop(writer);
        root.close().unwrap();
    }
}

#[test]
fn overlapping_or_same_folder_selection_is_refused_before_any_changes() {
    let (root, source, target) = fixture();
    let file = source.join("file.txt");
    fs::write(&file, b"contents").unwrap();
    for (paths, destination) in [
        (vec![source.clone(), file.clone()], target.clone()),
        (vec![file.clone()], source.clone()),
    ] {
        let result = crate::move_with_conflicts(&paths, &destination, &Progress::default());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidInput);
        assert_eq!(fs::read(&file).unwrap(), b"contents");
        assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
    }
    root.close().unwrap();
}

#[test]
fn reparse_source_is_refused_without_a_prompt_or_destination_writes() {
    let (root, source, target) = fixture();
    let protected = root.path().join("protected");
    fs::create_dir(&protected).unwrap();
    fs::write(protected.join("file.txt"), b"preserved").unwrap();
    let junction = source.join("link");
    assert!(std::process::Command::new("cmd.exe")
        .args(["/C", "mklink", "/J"])
        .arg(&junction)
        .arg(&protected)
        .output()
        .unwrap()
        .status
        .success());
    let (result, progress) = run(vec![source], target.clone(), |_| {
        panic!("Reparse paths must not prompt")
    });
    assert!(result.is_err());
    assert_eq!(fs::read(protected.join("file.txt")).unwrap(), b"preserved");
    assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
    assert_eq!(progress.files_moved.load(Ordering::Relaxed), 0);
    fs::remove_dir(junction).unwrap();
    root.close().unwrap();
}

#[test]
fn move_and_copy_share_the_fresh_progress_consent_boundary() {
    let (root, source, target) = fixture();
    let file = source.join("file.txt");
    fs::write(&file, b"contents").unwrap();
    let progress = Progress::default();
    let result =
        crate::move_with_conflicts(std::slice::from_ref(&file), &target, &progress).unwrap();
    assert_eq!(result.failures, 0);
    assert_eq!(
        crate::copy_with_conflicts(&[target.join("file.txt")], &source, &progress)
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(fs::read(target.join("file.txt")).unwrap(), b"contents");
    root.close().unwrap();
}
