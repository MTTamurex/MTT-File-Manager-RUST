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
    fs::write(source.join("file.txt"), b"contents").unwrap();
    let before = crate::replacement::Snapshot::capture(&source.join("file.txt")).unwrap();
    let (result, progress) = run(vec![source.clone()], target.clone(), |_| {
        panic!("No conflict")
    });
    let summary = result.unwrap();
    assert_eq!(summary.failures, 0, "{:?}", summary.first_error);
    assert_eq!(summary.files_moved, 1);
    assert!(!source.exists());
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
