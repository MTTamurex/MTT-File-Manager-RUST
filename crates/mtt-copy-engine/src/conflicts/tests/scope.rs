use super::*;

#[test]
fn apply_all_skips_or_replaces_files_but_cannot_be_reused_by_another_copy_call() {
    let (root, source, target) = fixture();
    let files: Vec<_> = (0..2)
        .map(|index| source.join(format!("file-{index}.txt")))
        .collect();
    for file in &files {
        fs::write(file, b"new").unwrap();
        fs::write(target.join(file.file_name().unwrap()), b"old").unwrap();
    }
    let (result, _, requests) = run(files.clone(), target.clone(), |_| {
        (ConflictAction::Skip, true)
    });
    assert_eq!(result.unwrap().items_skipped, 2);
    assert_eq!(requests, vec![ConflictKind::File]);
    for file in &files {
        assert_eq!(
            fs::read(target.join(file.file_name().unwrap())).unwrap(),
            b"old"
        );
    }
    let (result, progress, requests) = run(files.clone(), target.clone(), |_| {
        (ConflictAction::Replace, true)
    });
    assert_eq!(result.unwrap().files_copied, 2);
    assert_eq!(requests, vec![ConflictKind::File]);
    fs::write(&files[0], b"changed again").unwrap();
    assert_eq!(
        copy_with_conflicts(&files, &target, &progress)
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(fs::read(target.join("file-0.txt")).unwrap(), b"new");
    root.close().unwrap();
}

#[test]
fn folder_keep_both_and_skip_never_modify_the_existing_tree() {
    for action in [ConflictAction::KeepBoth, ConflictAction::Skip] {
        let (root, source, target) = fixture();
        fs::write(source.join("new.txt"), b"source").unwrap();
        fs::create_dir(target.join("source")).unwrap();
        fs::write(target.join("source/old.txt"), b"preserved").unwrap();
        let (result, _, requests) = run(vec![source.clone()], target.clone(), |_| (action, false));
        let summary = result.unwrap();
        assert_eq!(summary.failures, 0);
        assert_eq!(requests, vec![ConflictKind::Directory]);
        assert_eq!(
            fs::read(target.join("source/old.txt")).unwrap(),
            b"preserved"
        );
        assert!(!target.join("source/new.txt").exists());
        if action == ConflictAction::KeepBoth {
            assert_eq!(
                fs::read(target.join("source (2)/new.txt")).unwrap(),
                b"source"
            );
        } else {
            assert_eq!(summary.items_skipped, 1);
        }
        assert_eq!(fs::read(source.join("new.txt")).unwrap(), b"source");
        root.close().unwrap();
    }
}

#[test]
fn cancel_or_source_change_after_replacement_preflight_preserves_the_old_file() {
    for cancel in [false, true] {
        let (root, source, target) = fixture();
        let source_file = source.join("file.txt");
        fs::write(&source_file, b"new").unwrap();
        fs::write(target.join("file.txt"), b"old").unwrap();
        let progress = Arc::new(Progress::default());
        let control = progress.clone();
        let selected = source_file.clone();
        let destination = target.clone();
        let worker = std::thread::spawn(move || {
            crate::plan::build_interactive(&[selected], &destination, &control)
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(request) = progress.conflicts.pending() {
                assert!(progress
                    .conflicts
                    .respond(request.id, ConflictAction::Replace, false));
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        let plan = worker.join().unwrap().unwrap();
        if cancel {
            progress.cancel();
        } else {
            fs::write(&source_file, b"changed source").unwrap();
        }
        assert!(crate::platform::copy_file(&plan.files[0], &progress).is_err());
        assert_eq!(fs::read(target.join("file.txt")).unwrap(), b"old");
        assert_eq!(fs::read_dir(&target).unwrap().count(), 1);
        root.close().unwrap();
    }
}

#[cfg(windows)]
#[test]
fn replacement_guard_blocks_new_writers_until_released() {
    use std::os::windows::fs::OpenOptionsExt;
    use windows::Win32::Storage::FileSystem::{
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };
    let (root, _, target) = fixture();
    let file = target.join("file.txt");
    fs::write(&file, b"old").unwrap();
    let snapshot = crate::replacement::Snapshot::capture(&file).unwrap();
    let guard = snapshot.hold(&file).unwrap();
    let open_writer = || {
        fs::OpenOptions::new()
            .write(true)
            .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0)
            .open(&file)
    };
    assert_eq!(open_writer().unwrap_err().raw_os_error(), Some(32));
    drop(guard);
    drop(open_writer().unwrap());
    assert_eq!(fs::read(&file).unwrap(), b"old");
    root.close().unwrap();
}

#[cfg(windows)]
#[test]
fn an_open_writer_does_not_prevent_skip_or_keep_both_but_blocks_replacement() {
    use std::os::windows::fs::OpenOptionsExt;
    use windows::Win32::Storage::FileSystem::{
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };
    for action in [
        ConflictAction::Skip,
        ConflictAction::KeepBoth,
        ConflictAction::Replace,
    ] {
        let (root, source, target) = fixture();
        let file = source.join("file.txt");
        fs::write(&file, b"new").unwrap();
        fs::write(target.join("file.txt"), b"old").unwrap();
        let writer = fs::OpenOptions::new()
            .write(true)
            .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0)
            .open(target.join("file.txt"))
            .unwrap();
        let (result, progress, requests) = run(vec![file], target.clone(), |_| (action, false));
        assert_eq!(requests, vec![ConflictKind::File]);
        if action == ConflictAction::Replace {
            assert_eq!(result.unwrap_err().raw_os_error(), Some(32));
            assert_eq!(
                progress
                    .files_copied
                    .load(std::sync::atomic::Ordering::Relaxed),
                0
            );
            assert_eq!(
                progress
                    .bytes_copied
                    .load(std::sync::atomic::Ordering::Relaxed),
                0
            );
        } else {
            let summary = result.unwrap();
            assert_eq!(summary.failures, 0);
            assert_eq!(
                summary.files_copied,
                usize::from(action == ConflictAction::KeepBoth)
            );
            assert_eq!(
                summary.items_skipped,
                usize::from(action == ConflictAction::Skip)
            );
        }
        if action == ConflictAction::KeepBoth {
            assert_eq!(fs::read(target.join("file (2).txt")).unwrap(), b"new");
        }
        assert_eq!(
            fs::read_dir(&target).unwrap().count(),
            if action == ConflictAction::KeepBoth {
                2
            } else {
                1
            }
        );
        assert_eq!(fs::read(target.join("file.txt")).unwrap(), b"old");
        drop(writer);
        root.close().unwrap();
    }
}
