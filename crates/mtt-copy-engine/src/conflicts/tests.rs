use super::*;
use crate::{copy_with_conflicts, Summary};
use std::fs;
use std::sync::Arc;
use std::time::Instant;
mod protocol;
mod scope;

fn run(
    sources: Vec<PathBuf>,
    destination: PathBuf,
    mut decide: impl FnMut(&Conflict) -> (ConflictAction, bool),
) -> (io::Result<Summary>, Arc<Progress>, Vec<ConflictKind>) {
    let progress = Arc::new(Progress::default());
    let control = progress.clone();
    let worker = std::thread::spawn(move || copy_with_conflicts(&sources, &destination, &control));
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut answered = Vec::new();
    while !worker.is_finished() && Instant::now() < deadline {
        if let Some(request) = progress.conflicts.pending() {
            let (action, all) = decide(&request);
            if progress.conflicts.respond(request.id, action, all) {
                answered.push(request.kind);
            }
        }
        std::thread::yield_now();
    }
    if !worker.is_finished() {
        progress.cancel();
    }
    (worker.join().unwrap(), progress, answered)
}

fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&target).unwrap();
    (root, source, target)
}

#[test]
fn replace_skip_and_keep_both_preserve_source_and_only_publish_verified_copies() {
    for action in [
        ConflictAction::Replace,
        ConflictAction::Skip,
        ConflictAction::KeepBoth,
    ] {
        let (root, source, target) = fixture();
        let file = source.join("file.txt");
        fs::write(&file, b"new source contents").unwrap();
        fs::write(target.join("file.txt"), b"old target").unwrap();
        let (result, progress, requests) =
            run(vec![file.clone()], target.clone(), |_| (action, false));
        let summary = result.unwrap();
        assert_eq!(summary.failures, 0, "{action:?}: {:?}", summary.first_error);
        assert_eq!(requests, vec![ConflictKind::File]);
        assert_eq!(fs::read(&file).unwrap(), b"new source contents");
        let replaced = action == ConflictAction::Replace;
        assert_eq!(
            fs::read(target.join("file.txt")).unwrap(),
            if replaced {
                b"new source contents".as_slice()
            } else {
                b"old target".as_slice()
            }
        );
        if action == ConflictAction::KeepBoth {
            assert_eq!(
                fs::read(target.join("file (2).txt")).unwrap(),
                b"new source contents"
            );
        }
        assert_eq!(
            summary.items_skipped,
            usize::from(action == ConflictAction::Skip)
        );
        assert_eq!(
            progress
                .files_copied
                .load(std::sync::atomic::Ordering::Relaxed),
            usize::from(action != ConflictAction::Skip)
        );
        assert_eq!(
            fs::read_dir(&target).unwrap().count(),
            if action == ConflictAction::KeepBoth {
                2
            } else {
                1
            }
        );
        root.close().unwrap();
    }
}

#[test]
fn folder_merge_preserves_unrelated_contents_and_file_decisions_have_separate_scope() {
    let (root, source, target) = fixture();
    fs::create_dir_all(source.join("nested/empty")).unwrap();
    fs::write(source.join("nested/file.txt"), b"new").unwrap();
    fs::write(source.join("new.txt"), b"added").unwrap();
    fs::create_dir_all(target.join("source/nested")).unwrap();
    fs::write(target.join("source/nested/file.txt"), b"old").unwrap();
    fs::write(target.join("source/unrelated.txt"), b"preserved").unwrap();
    let (result, _, requests) = run(vec![source.clone()], target.clone(), |request| {
        (
            if request.kind == ConflictKind::Directory {
                ConflictAction::Merge
            } else {
                ConflictAction::Replace
            },
            true,
        )
    });
    let summary = result.unwrap();
    assert_eq!(summary.failures, 0, "{:?}", summary.first_error);
    assert_eq!(requests, vec![ConflictKind::Directory, ConflictKind::File]);
    assert_eq!(
        fs::read(target.join("source/nested/file.txt")).unwrap(),
        b"new"
    );
    assert_eq!(
        fs::read(target.join("source/unrelated.txt")).unwrap(),
        b"preserved"
    );
    assert!(target.join("source/nested/empty").is_dir());
    root.close().unwrap();
}

#[test]
fn keep_both_reserves_names_and_same_folder_copy_never_replaces_the_source() {
    let (root, source, target) = fixture();
    fs::write(source.join("file.txt"), b"original").unwrap();
    let (result, _, requests) = run(vec![source.join("file.txt")], source.clone(), |request| {
        assert!(!request.allows(ConflictAction::Replace));
        (ConflictAction::KeepBoth, false)
    });
    assert_eq!(result.unwrap().failures, 0);
    assert_eq!(requests, vec![ConflictKind::SamePath]);
    assert_eq!(fs::read(source.join("file.txt")).unwrap(), b"original");
    assert_eq!(fs::read(source.join("file (2).txt")).unwrap(), b"original");
    let other = root.path().join("other");
    fs::create_dir(&other).unwrap();
    fs::write(other.join("file.txt"), b"other").unwrap();
    let (result, _, _) = run(
        vec![source.join("file.txt"), other.join("file.txt")],
        target.clone(),
        |_| (ConflictAction::KeepBoth, true),
    );
    assert_eq!(result.unwrap().failures, 0);
    assert_eq!(fs::read(target.join("file.txt")).unwrap(), b"original");
    assert_eq!(fs::read(target.join("file (2).txt")).unwrap(), b"other");
    root.close().unwrap();
}

#[test]
fn cancel_and_changed_target_after_prompt_never_replace_old_data() {
    for change_target in [false, true] {
        let (root, source, target) = fixture();
        fs::write(source.join("file.txt"), b"new").unwrap();
        let existing = target.join("file.txt");
        fs::write(&existing, b"old").unwrap();
        let (result, _, _) = run(vec![source.join("file.txt")], target.clone(), |_| {
            if change_target {
                fs::write(&existing, b"changed externally").unwrap();
            }
            (
                if change_target {
                    ConflictAction::Replace
                } else {
                    ConflictAction::Cancel
                },
                false,
            )
        });
        assert!(result.is_err());
        assert_eq!(
            fs::read(&existing).unwrap(),
            if change_target {
                b"changed externally".as_slice()
            } else {
                b"old".as_slice()
            }
        );
        assert_eq!(fs::read_dir(&target).unwrap().count(), 1);
        root.close().unwrap();
    }
}

#[cfg(windows)]
#[test]
fn readonly_replacement_failure_keeps_old_file_and_cleans_staging() {
    let (root, source, target) = fixture();
    fs::write(source.join("file.txt"), b"new").unwrap();
    let existing = target.join("file.txt");
    fs::write(&existing, b"old").unwrap();
    let mut permissions = fs::metadata(&existing).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&existing, permissions).unwrap();
    let (result, progress, _) = run(vec![source.join("file.txt")], target.clone(), |_| {
        (ConflictAction::Replace, false)
    });
    assert_eq!(result.unwrap().failures, 1);
    assert_eq!(fs::read(&existing).unwrap(), b"old");
    assert_eq!(
        progress
            .bytes_copied
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
    assert_eq!(fs::read_dir(&target).unwrap().count(), 1);
    let mut permissions = fs::metadata(&existing).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    fs::set_permissions(&existing, permissions).unwrap();
    root.close().unwrap();
}

#[test]
fn type_mismatch_cannot_authorize_destruction_of_a_directory() {
    let (root, source, target) = fixture();
    fs::write(source.join("file.txt"), b"new").unwrap();
    fs::create_dir(target.join("file.txt")).unwrap();
    fs::write(target.join("file.txt/child.txt"), b"preserved").unwrap();
    let (result, _, _) = run(vec![source.join("file.txt")], target.clone(), |request| {
        assert_eq!(request.kind, ConflictKind::TypeMismatch);
        assert!(!request.allows(ConflictAction::Replace));
        (ConflictAction::Skip, false)
    });
    assert_eq!(result.unwrap().items_skipped, 1);
    assert_eq!(
        fs::read(target.join("file.txt/child.txt")).unwrap(),
        b"preserved"
    );
    root.close().unwrap();
}

#[cfg(windows)]
#[test]
fn reparse_destination_is_refused_before_any_conflict_question() {
    let (root, source, target) = fixture();
    fs::write(source.join("file.txt"), b"new").unwrap();
    let protected = root.path().join("protected");
    fs::create_dir(&protected).unwrap();
    fs::write(protected.join("file.txt"), b"preserved").unwrap();
    let junction = target.join("source");
    assert!(std::process::Command::new("cmd.exe")
        .args(["/C", "mklink", "/J"])
        .arg(&junction)
        .arg(&protected)
        .output()
        .unwrap()
        .status
        .success());
    let (result, _, requests) = run(vec![source], target, |_| {
        panic!("Reparse paths must not prompt")
    });
    assert!(result.is_err());
    assert!(requests.is_empty());
    assert_eq!(fs::read(protected.join("file.txt")).unwrap(), b"preserved");
    fs::remove_dir(junction).unwrap();
    root.close().unwrap();
}
