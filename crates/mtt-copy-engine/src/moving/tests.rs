use super::*;
use crate::conflicts::{ConflictAction, ConflictKind};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
mod safety;

fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&target).unwrap();
    (root, source, target)
}

fn run(
    sources: Vec<PathBuf>,
    destination: PathBuf,
    mut action: impl FnMut(ConflictKind) -> ConflictAction,
) -> (io::Result<Summary>, Arc<Progress>) {
    let progress = Arc::new(Progress::default());
    let control = progress.clone();
    let worker =
        std::thread::spawn(move || crate::move_with_conflicts(&sources, &destination, &control));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !worker.is_finished() && Instant::now() < deadline {
        if let Some(request) = progress.conflicts.pending() {
            progress
                .conflicts
                .respond(request.id, action(request.kind), true);
        }
        std::thread::yield_now();
    }
    if !worker.is_finished() {
        progress.cancel();
    }
    (worker.join().unwrap(), progress)
}

#[test]
fn moves_files_by_rename_and_cleans_only_empty_source_directories() {
    let (root, source, target) = fixture();
    fs::create_dir(source.join("empty")).unwrap();
    let other = root.path().join("other");
    fs::create_dir(&other).unwrap();
    fs::write(source.join("file.txt"), b"contents").unwrap();
    let before = crate::replacement::Snapshot::capture(&source.join("file.txt")).unwrap();
    let (result, progress) = run(vec![source.clone(), other.clone()], target.clone(), |_| {
        panic!("No conflict")
    });
    let summary = result.unwrap();
    assert_eq!(summary.failures, 0, "{:?}", summary.first_error);
    assert_eq!(summary.files_moved, 1);
    assert_eq!(summary.directories_moved, 3);
    assert_eq!(
        progress.telemetry.move_strategy(),
        crate::MoveStrategy::PerEntry
    );
    assert!(!source.exists());
    assert!(!other.exists());
    assert!(target.join("other").is_dir());
    assert!(target.join("source/empty").is_dir());
    assert_eq!(
        fs::read(target.join("source/file.txt")).unwrap(),
        b"contents"
    );
    assert_eq!(
        before,
        crate::replacement::Snapshot::capture(&target.join("source/file.txt")).unwrap()
    );
    assert_eq!(progress.telemetry.transferred.load(Ordering::Relaxed), 0);
    root.close().unwrap();
}

#[test]
fn conflicts_replace_skip_keep_both_and_merge_without_removing_skipped_items() {
    for action in [
        ConflictAction::Replace,
        ConflictAction::Skip,
        ConflictAction::KeepBoth,
    ] {
        let (root, source, target) = fixture();
        fs::write(source.join("file.txt"), b"new").unwrap();
        fs::write(target.join("file.txt"), b"old").unwrap();
        let (result, _) = run(vec![source.join("file.txt")], target.clone(), |_| action);
        let summary = result.unwrap();
        assert_eq!(summary.failures, 0, "{:?}", summary.first_error);
        assert_eq!(
            summary.files_moved,
            usize::from(action != ConflictAction::Skip)
        );
        assert_eq!(
            source.join("file.txt").exists(),
            action == ConflictAction::Skip
        );
        assert_eq!(
            fs::read(target.join("file.txt")).unwrap(),
            if action == ConflictAction::Replace {
                b"new"
            } else {
                b"old"
            }
        );
        if action == ConflictAction::KeepBoth {
            assert_eq!(fs::read(target.join("file (2).txt")).unwrap(), b"new");
        }
        root.close().unwrap();
    }
    let (root, source, target) = fixture();
    fs::write(source.join("skip.txt"), b"source").unwrap();
    fs::write(source.join("move.txt"), b"moved").unwrap();
    fs::create_dir(target.join("source")).unwrap();
    fs::write(target.join("source/skip.txt"), b"old").unwrap();
    let (result, _) = run(vec![source.clone()], target.clone(), |kind| {
        if kind == ConflictKind::Directory {
            ConflictAction::Merge
        } else {
            ConflictAction::Skip
        }
    });
    let summary = result.unwrap();
    assert_eq!(summary.failures, 0, "{:?}", summary.first_error);
    assert_eq!(summary.items_skipped, 1);
    assert_eq!(fs::read(source.join("skip.txt")).unwrap(), b"source");
    assert!(!source.join("move.txt").exists());
    assert_eq!(fs::read(target.join("source/move.txt")).unwrap(), b"moved");
    root.close().unwrap();
}

#[test]
fn moves_a_directory_tree_atomically_without_scanning_its_files() {
    let (root, source, target) = fixture();
    let nested = source.join("nested").join("deeper");
    fs::create_dir_all(&nested).unwrap();
    let file_paths = [source.join("root.bin"), nested.join("nested.bin")];
    fs::write(&file_paths[0], [0, 1, 2, 3, 254, 255]).unwrap();
    fs::write(&file_paths[1], b"nested file contents").unwrap();
    let before: Vec<_> = file_paths
        .iter()
        .map(|path| crate::replacement::Snapshot::capture(path).unwrap())
        .collect();
    let progress = Progress::default();
    progress.telemetry.enable_detailed_timings();

    let summary =
        crate::move_with_conflicts(std::slice::from_ref(&source), &target, &progress).unwrap();

    let moved_root = target.join("source");
    assert_eq!(summary.failures, 0, "{:?}", summary.first_error);
    assert_eq!(
        summary.files_moved, 0,
        "fast move must not invent file counts"
    );
    assert_eq!(summary.directories_moved, 1);
    assert_eq!(
        summary.fast_moved_directory,
        Some((source.clone(), moved_root.clone()))
    );
    assert_eq!(
        summary.moved_pairs,
        vec![(source.clone(), moved_root.clone())]
    );
    assert_eq!(summary.destinations, vec![moved_root.clone()]);
    assert_eq!(progress.total_files.load(Ordering::Relaxed), 0);
    assert_eq!(progress.total_bytes.load(Ordering::Relaxed), 0);
    assert_eq!(progress.directories_moved.load(Ordering::Relaxed), 1);
    assert_eq!(
        progress.telemetry.move_strategy(),
        crate::MoveStrategy::FastDirectoryRename
    );
    assert_eq!(
        progress.telemetry.inventory_snapshot(),
        crate::telemetry::InventorySnapshot::default()
    );
    assert!(!source.exists());
    assert!(moved_root.join("nested/deeper").is_dir());
    for (index, original) in file_paths.iter().enumerate() {
        let relative = original.strip_prefix(&source).unwrap();
        let moved = moved_root.join(relative);
        assert_eq!(
            crate::replacement::Snapshot::capture(&moved).unwrap(),
            before[index],
            "atomic rename must preserve file identity and metadata"
        );
        assert_eq!(
            fs::read(moved).unwrap(),
            if index == 0 {
                &[0, 1, 2, 3, 254, 255][..]
            } else {
                b"nested file contents"
            }
        );
    }
    root.close().unwrap();
}

#[test]
fn cross_volume_directory_rename_falls_back_without_changing_the_source() {
    let (root, source, target) = fixture();
    fs::create_dir(source.join("nested")).unwrap();
    let file = source.join("nested/file.bin");
    fs::write(&file, b"source remains intact").unwrap();
    let before = crate::replacement::Snapshot::capture(&file).unwrap();
    let progress = Progress::default();
    progress.telemetry.enable_detailed_timings();

    let result = try_rename_single_directory_with(
        std::slice::from_ref(&source),
        &target,
        &progress,
        |_, _| Err(io::Error::from_raw_os_error(17)),
    )
    .unwrap();

    assert!(
        result.is_none(),
        "cross-volume must use the existing pipeline"
    );
    assert!(source.is_dir());
    assert!(!target.join("source").exists());
    assert_eq!(fs::read(&file).unwrap(), b"source remains intact");
    assert_eq!(
        crate::replacement::Snapshot::capture(&file).unwrap(),
        before
    );
    assert_eq!(progress.directories_moved.load(Ordering::Relaxed), 0);
    assert_eq!(progress.telemetry.cross_volume_directory_fallbacks(), 1);
    root.close().unwrap();
}

#[test]
fn non_volume_directory_rename_failure_falls_back_without_changing_the_source() {
    let (root, source, target) = fixture();
    let file = source.join("nested/file.bin");
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, b"source remains intact").unwrap();
    let before = crate::replacement::Snapshot::capture(&file).unwrap();
    let progress = Progress::default();

    let result = try_rename_single_directory_with(
        std::slice::from_ref(&source),
        &target,
        &progress,
        |_, _| Err(io::Error::from(io::ErrorKind::PermissionDenied)),
    )
    .unwrap();

    assert!(
        result.is_none(),
        "non-volume rename failures must use the existing move pipeline"
    );
    assert!(source.is_dir());
    assert!(!target.join("source").exists());
    assert_eq!(fs::read(&file).unwrap(), b"source remains intact");
    assert_eq!(
        crate::replacement::Snapshot::capture(&file).unwrap(),
        before
    );
    root.close().unwrap();
}

#[test]
fn cancellation_before_directory_rename_leaves_the_tree_untouched() {
    let (root, source, target) = fixture();
    let file = source.join("file.bin");
    fs::write(&file, b"source data").unwrap();
    let progress = Progress::default();
    progress.cancel();

    let result = try_rename_single_directory_with(
        std::slice::from_ref(&source),
        &target,
        &progress,
        |_, _| panic!("cancelled operation must not rename"),
    );

    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Interrupted);
    assert_eq!(fs::read(&file).unwrap(), b"source data");
    assert!(!target.join("source").exists());
    root.close().unwrap();
}

#[test]
fn a_target_created_before_the_atomic_rename_is_never_replaced() {
    let (root, source, target) = fixture();
    let file = source.join("file.bin");
    fs::write(&file, b"source data").unwrap();
    let destination = target.join("source");
    let progress = Progress::default();

    let result = try_rename_single_directory_with(
        std::slice::from_ref(&source),
        &target,
        &progress,
        |handle, path| {
            fs::create_dir(path)?;
            fs::write(path.join("marker.bin"), b"preexisting destination")?;
            source::rename(handle, path, false)
        },
    );

    assert!(
        result.unwrap().is_none(),
        "a late destination collision must fall back to conflict handling"
    );
    assert_eq!(fs::read(&file).unwrap(), b"source data");
    assert_eq!(
        fs::read(destination.join("marker.bin")).unwrap(),
        b"preexisting destination"
    );
    assert_eq!(progress.directories_moved.load(Ordering::Relaxed), 0);
    root.close().unwrap();
}

fn one_file_plan(
    source: &std::path::Path,
    target: &std::path::Path,
    progress: &Progress,
) -> plan::Plan {
    plan::build_move(&[source.to_owned()], target, progress).unwrap()
}

#[test]
fn forced_cross_volume_path_copies_verifies_and_removes_source() {
    let (root, source, target) = fixture();
    let file = source.join("file.txt");
    fs::write(&file, b"contents").unwrap();
    let progress = Progress::default();
    let plan = one_file_plan(&file, &target, &progress);
    let result = transfer_with(
        &plan.files[0],
        &progress,
        None,
        CacheMode::Buffered,
        true,
        || Ok(()),
    )
    .unwrap();
    assert!(result.is_none(), "{result:?}");
    assert!(!file.exists());
    assert_eq!(fs::read(target.join("file.txt")).unwrap(), b"contents");
    assert_eq!(fs::read_dir(&target).unwrap().count(), 1);
    root.close().unwrap();
}

#[test]
fn publication_then_cancellation_or_cleanup_failure_retains_source_and_copy() {
    for cancel in [false, true] {
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
                if cancel {
                    progress.cancel();
                    Ok(())
                } else {
                    Err(io::ErrorKind::PermissionDenied.into())
                }
            },
        )
        .unwrap();
        assert!(retained.is_some());
        assert_eq!(fs::read(&file).unwrap(), b"contents");
        assert_eq!(fs::read(target.join("file.txt")).unwrap(), b"contents");
        assert_eq!(fs::read_dir(&target).unwrap().count(), 1);
        root.close().unwrap();
    }
}

#[test]
fn changed_source_or_preflight_cancellation_never_removes_source() {
    for cancel in [false, true] {
        let (root, source, target) = fixture();
        let file = source.join("file.txt");
        fs::write(&file, b"contents").unwrap();
        let progress = Progress::default();
        let plan = one_file_plan(&file, &target, &progress);
        if cancel {
            progress.cancel();
        } else {
            fs::write(&file, b"changed").unwrap();
        }
        assert!(transfer(&plan.files[0], &progress, None, CacheMode::Buffered).is_err());
        assert!(file.exists());
        assert!(!target.join("file.txt").exists());
        root.close().unwrap();
    }
}
