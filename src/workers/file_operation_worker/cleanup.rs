//! Cleanup deletion handler
//!
//! Executes a cleanup run on a worker STA-COM thread: silent per-item shell
//! deletes with per-item revalidation. Safety layers, in order:
//! 1. Every path re-passes `sanitize_operation_paths` (drive whitelist, UNC,
//!    device namespace rejection).
//! 2. Every item is re-statted: missing = already gone (success), reparse
//!    point = skipped, modified since the scan cutoff = skipped (stale
//!    rejection — the user confirmed a snapshot, not the live filesystem).
//! 3. Deletion is per item with no native UI, so one locked file becomes a
//!    skip, never a dialog storm or an aborted batch.

use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use super::{FileOperationResult, SendHwnd};

/// Never follow (or delete) reparse points; junctions need the raw bit.
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

pub(crate) struct CleanupDeleteContext<'a> {
    pub paths: Vec<PathBuf>,
    pub permanent: bool,
    pub cutoff_unix: u64,
    pub cancel: Arc<AtomicBool>,
    pub hwnd: SendHwnd,
    pub result_sender: &'a std::sync::mpsc::Sender<FileOperationResult>,
}

enum CandidateStatus {
    AlreadyGone,
    Stale(&'static str),
    Ready,
}

pub(crate) fn handle_cleanup_delete(context: CleanupDeleteContext<'_>) {
    let sanitized = match super::sanitize_operation_paths(&context.paths) {
        Ok(paths) => paths,
        Err(error) => {
            log::warn!("[Cleanup] Path sanitization rejected the batch: {error}");
            let _ = context.result_sender.send(FileOperationResult::CleanupDeleteCompleted {
                deleted_paths: Vec::new(),
                failed: context
                    .paths
                    .iter()
                    .map(|path| (path.clone(), "rejected".to_string()))
                    .collect(),
                aborted: false,
            });
            return;
        }
    };

    let total = sanitized.len();
    let mut deleted_paths: Vec<PathBuf> = Vec::new();
    let mut failed: Vec<(PathBuf, String)> = Vec::new();

    for (index, path) in sanitized.iter().enumerate() {
        if context.cancel.load(Ordering::Relaxed) {
            for remaining in &sanitized[index..] {
                failed.push((remaining.clone(), "cancelled".to_string()));
            }
            send_completed(context.result_sender, deleted_paths, failed, true);
            return;
        }

        match evaluate_candidate(path, context.cutoff_unix) {
            CandidateStatus::AlreadyGone => deleted_paths.push(path.clone()),
            CandidateStatus::Stale(reason) => failed.push((path.clone(), reason.to_string())),
            CandidateStatus::Ready => {
                match crate::infrastructure::windows::shell_operations::delete_single_silent(
                    path,
                    context.hwnd.0,
                    !context.permanent,
                ) {
                    Ok(()) => deleted_paths.push(path.clone()),
                    Err(code) => failed.push((path.clone(), format!("shell_error_{code}"))),
                }
            }
        }

        let _ = context.result_sender.send(FileOperationResult::CleanupDeleteProgress {
            completed: index + 1,
            total,
            current_name: path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned()),
            failed_count: failed.len(),
        });
    }

    send_completed(context.result_sender, deleted_paths, failed, false);
}

fn send_completed(
    result_sender: &std::sync::mpsc::Sender<FileOperationResult>,
    deleted_paths: Vec<PathBuf>,
    failed: Vec<(PathBuf, String)>,
    aborted: bool,
) {
    let _ = result_sender.send(FileOperationResult::CleanupDeleteCompleted {
        deleted_paths,
        failed,
        aborted,
    });
}

/// Re-validates one scanned candidate at delete time.
fn evaluate_candidate(path: &Path, cutoff_unix: u64) -> CandidateStatus {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => CandidateStatus::AlreadyGone,
        Err(error) => {
            log::debug!("[Cleanup] '{}' unavailable: {}", path.display(), error);
            CandidateStatus::Stale("unavailable")
        }
        Ok(metadata) => {
            if metadata.file_type().is_symlink()
                || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            {
                return CandidateStatus::Stale("reparse_point");
            }
            let modified_unix = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs())
                .unwrap_or(0);
            if modified_unix >= cutoff_unix {
                CandidateStatus::Stale("modified_since_scan")
            } else {
                CandidateStatus::Ready
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    const HOUR: u64 = 3600;

    fn temp_file(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, b"data").expect("write test file");
        path
    }

    /// Sets a file's mtime to `hours_ago` hours in the past.
    fn backdate_file(path: &Path, hours_ago: u64) {
        let past = SystemTime::now() - Duration::from_secs(hours_ago * HOUR);
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .expect("open for backdating");
        file.set_modified(past).expect("set_modified");
    }

    fn day_cutoff() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0)
            .saturating_sub(24 * HOUR)
    }

    #[test]
    fn missing_item_is_already_gone() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("ghost.txt");
        assert!(matches!(
            evaluate_candidate(&path, 0),
            CandidateStatus::AlreadyGone
        ));
    }

    #[test]
    fn old_item_older_than_cutoff_is_ready() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = temp_file(directory.path(), "item.txt");
        backdate_file(&path, 48);
        assert!(matches!(
            evaluate_candidate(&path, day_cutoff()),
            CandidateStatus::Ready
        ));
    }

    #[test]
    fn item_modified_after_cutoff_is_stale() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = temp_file(directory.path(), "fresh.txt");
        // Written just now: newer than a 24 h cutoff.
        assert!(matches!(
            evaluate_candidate(&path, day_cutoff()),
            CandidateStatus::Stale("modified_since_scan")
        ));
    }

    #[test]
    fn reparse_point_item_is_stale() {
        let directory = tempfile::tempdir().expect("tempdir");
        let target = temp_file(directory.path(), "target.txt");
        let link = directory.path().join("link.txt");
        match std::os::windows::fs::symlink_file(&target, &link) {
            Ok(()) => assert!(matches!(
                evaluate_candidate(&link, 0),
                CandidateStatus::Stale("reparse_point")
            )),
            Err(error) if error.raw_os_error() == Some(1314) => {}
            Err(error) => panic!("unexpected symlink creation failure: {error}"),
        }
    }
}
