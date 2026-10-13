//! Read-only cleanup scanner
//!
//! Walks the built-in category roots and produces first-level deletion
//! candidates. The scanner never writes, never follows reparse points, and
//! applies the age filter conservatively: a directory only qualifies when
//! nothing inside it was modified after the cutoff, so deleting the folder
//! can never take a recently-touched file with it.

use crate::domain::cleanup::{CleanupCandidate, CleanupCategory, SharedCleanupScanProgress};
use std::os::windows::fs::MetadataExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::UNIX_EPOCH;

/// Never follow (or delete) reparse points. Junctions are not reported as
/// symlinks by `std`, so the raw attribute bit is the only reliable signal.
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

/// Hard cap on candidates sent to the UI. First-level entries of temp/cache
/// roots are expected in the hundreds; this only guards pathological systems.
const MAX_CANDIDATES: usize = 10_000;

/// Depth limit for measuring folder sizes.
const MAX_MEASURE_DEPTH: usize = 16;

/// Final message from the scan worker to the UI thread. The generation id
/// lets the UI reject results from a superseded scan job.
#[derive(Debug)]
pub enum CleanupScanMessage {
    Completed {
        generation: u64,
        candidates: Vec<CleanupCandidate>,
    },
    Failed {
        generation: u64,
        error: String,
    },
}

/// Everything the scan job needs. `cutoff_unix` = "modified before this
/// moment" (unix seconds): only entries strictly older than the cutoff
/// qualify.
pub struct CleanupScanJob {
    pub categories: Vec<CleanupCategory>,
    pub cutoff_unix: u64,
    pub cancel: Arc<AtomicBool>,
    pub progress: SharedCleanupScanProgress,
    pub sender: Sender<CleanupScanMessage>,
    pub generation: u64,
}

/// Runs the whole scan synchronously; call from a worker thread via
/// `spawn_named`. Always sends exactly one message (Completed or Failed).
pub fn run_cleanup_scan(job: CleanupScanJob) {
    let generation = job.generation;
    let result = scan_all(&job);
    let message = match result {
        Ok(candidates) => CleanupScanMessage::Completed {
            generation,
            candidates,
        },
        Err(error) => CleanupScanMessage::Failed { generation, error },
    };
    let _ = job.sender.send(message);
}

fn scan_all(job: &CleanupScanJob) -> Result<Vec<CleanupCandidate>, String> {
    let mut candidates = Vec::new();
    'outer: for category in &job.categories {
        for root in &category.roots {
            scan_root(job, category, root, &mut candidates);
            if candidates.len() >= MAX_CANDIDATES || job.cancel.load(Ordering::Relaxed) {
                break 'outer;
            }
        }
    }
    Ok(candidates)
}

fn scan_root(
    job: &CleanupScanJob,
    category: &CleanupCategory,
    root: &Path,
    candidates: &mut Vec<CleanupCandidate>,
) {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        // Unreadable root (missing, no permission): no candidates, not fatal.
        Err(error) => {
            log::debug!("[Cleanup] Root '{}' unreadable: {}", root.display(), error);
            return;
        }
    };

    for entry in entries.flatten() {
        if candidates.len() >= MAX_CANDIDATES || job.cancel.load(Ordering::Relaxed) {
            return;
        }
        let path = entry.path();
        // Containment guard: the entry must remain inside the declared root.
        if !category.contains_path(&path) {
            continue;
        }
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        if metadata.file_type().is_symlink()
            || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        {
            continue;
        }
        let modified_unix = system_time_to_unix(metadata.modified().ok());
        // Pattern-restricted roots only accept matching first-level files.
        let passes_pattern = match &category.file_patterns {
            Some(patterns) => {
                !metadata.is_dir() && file_matches_patterns(&path, patterns.as_slice())
            }
            None => true,
        };
        if !passes_pattern {
            continue;
        }

        let candidate = if metadata.is_dir() {
            let Some((size, newest_descendant)) =
                measure_directory(&path, job, job.cutoff_unix, 0)
            else {
                continue;
            };
            // Conservative: the folder's own mtime must also be older than
            // the cutoff — a recently active folder may still be in use.
            let effective_modified = modified_unix.max(newest_descendant);
            if effective_modified >= job.cutoff_unix {
                continue;
            }
            CleanupCandidate {
                path,
                category_id: category.id,
                size,
                is_dir: true,
                modified_unix: effective_modified,
            }
        } else {
            if modified_unix >= job.cutoff_unix {
                continue;
            }
            CleanupCandidate {
                path,
                category_id: category.id,
                size: metadata.len(),
                is_dir: false,
                modified_unix,
            }
        };

        candidates.push(candidate);
        publish_progress(job, candidates);
    }
}

/// Recursively measures a folder: total size plus the newest mtime found
/// anywhere inside. Returns `None` when the folder must not be a candidate
/// (reparse point at the top, or anything inside was modified after the
/// cutoff — conservative: skip rather than risk recent data).
fn measure_directory(
    dir: &Path,
    job: &CleanupScanJob,
    cutoff_unix: u64,
    depth: usize,
) -> Option<(u64, u64)> {
    if depth > MAX_MEASURE_DEPTH || job.cancel.load(Ordering::Relaxed) {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    let mut total = 0_u64;
    let mut newest = 0_u64;
    for entry in entries.flatten() {
        if job.cancel.load(Ordering::Relaxed) {
            return None;
        }
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        if metadata.file_type().is_symlink()
            || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        {
            continue;
        }
        let modified_unix = system_time_to_unix(metadata.modified().ok());
        if metadata.is_dir() {
            let (child_size, child_newest) =
                measure_directory(&entry.path(), job, cutoff_unix, depth + 1)?;
            total = total.saturating_add(child_size);
            newest = newest.max(child_newest);
        } else {
            total = total.saturating_add(metadata.len());
            newest = newest.max(modified_unix);
        }
        if newest >= cutoff_unix {
            // Something inside is too recent: the folder never qualifies and
            // continuing to measure only wastes I/O.
            return None;
        }
    }
    Some((total, newest))
}

fn file_matches_patterns(path: &Path, patterns: &[&str]) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let lowered = name.to_ascii_lowercase();
    patterns.iter().any(|pattern| lowered.starts_with(pattern))
}

fn system_time_to_unix(time: Option<std::time::SystemTime>) -> u64 {
    time.and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn publish_progress(job: &CleanupScanJob, candidates: &[CleanupCandidate]) {
    if let Ok(mut progress) = job.progress.lock() {
        progress.entries_seen = candidates.len() as u64;
        progress.bytes_found = candidates.iter().map(|candidate| candidate.size).sum();
        progress.current_path = candidates
            .last()
            .map(|candidate| candidate.path.to_string_lossy().into_owned())
            .unwrap_or_default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::cleanup::{CleanupCategoryId, CleanupScanProgress};
    use std::path::PathBuf;
    use std::sync::mpsc::channel;
    use std::time::{Duration, SystemTime};

    const HOUR: u64 = 3600;

    /// Age cutoff matching a 24 h filter.
    fn day_cutoff() -> u64 {
        now_unix() - 24 * HOUR
    }

    fn now_unix() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0)
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

    fn make_job(
        cutoff: u64,
        cancel: Arc<AtomicBool>,
        categories: Vec<CleanupCategory>,
    ) -> CleanupScanJob {
        // The scan itself never uses the sender; only run_cleanup_scan does.
        let (sender, _receiver) = channel();
        CleanupScanJob {
            categories,
            cutoff_unix: cutoff,
            cancel,
            progress: Arc::new(std::sync::Mutex::new(CleanupScanProgress::default())),
            sender,
            generation: 0,
        }
    }

    fn category_for(root: PathBuf) -> CleanupCategory {
        CleanupCategory {
            id: CleanupCategoryId::UserTemp,
            roots: vec![root],
            file_patterns: None,
            requires_admin: false,
        }
    }

    fn cancel_flag() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    #[test]
    fn empty_category_list_yields_no_candidates() {
        let job = make_job(u64::MAX, cancel_flag(), Vec::new());
        let candidates = scan_all(&job).expect("scan should not fail");
        assert!(candidates.is_empty());
    }

    #[test]
    fn cutoff_zero_rejects_every_item() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join("file.txt"), b"data").expect("write");

        let job = make_job(
            0,
            cancel_flag(),
            vec![category_for(temp.path().to_path_buf())],
        );
        let candidates = scan_all(&job).expect("scan should not fail");
        assert!(candidates.is_empty(), "cutoff 0 means every mtime is too new");
    }

    #[test]
    fn only_old_files_qualify_with_age_cutoff() {
        let temp = tempfile::tempdir().expect("tempdir");
        let old_file = temp.path().join("old.txt");
        std::fs::write(&old_file, b"old").expect("write");
        backdate_file(&old_file, 48);
        std::fs::write(temp.path().join("fresh.txt"), b"fresh").expect("write");

        let job = make_job(
            day_cutoff(),
            cancel_flag(),
            vec![category_for(temp.path().to_path_buf())],
        );
        let candidates = scan_all(&job).expect("scan should not fail");

        assert_eq!(candidates.len(), 1, "only the backdated file qualifies");
        assert_eq!(candidates[0].path.file_name().unwrap(), "old.txt");
        assert_eq!(candidates[0].size, 3);
    }

    #[test]
    fn folder_with_fresh_mtime_is_not_candidate() {
        let temp = tempfile::tempdir().expect("tempdir");
        let dir = temp.path().join("old_content_folder");
        std::fs::create_dir(&dir).expect("mkdir");
        let inner = dir.join("inner.bin");
        std::fs::write(&inner, b"old").expect("write");
        backdate_file(&inner, 48);

        // Conservative rule: the folder's own mtime (just created = now)
        // must also be older than the cutoff, so no directory candidate.
        let job = make_job(
            day_cutoff(),
            cancel_flag(),
            vec![category_for(temp.path().to_path_buf())],
        );
        let candidates = scan_all(&job).expect("scan should not fail");
        assert!(
            candidates.iter().all(|candidate| !candidate.is_dir),
            "folder with fresh mtime must not be a candidate"
        );
    }

    #[test]
    fn measure_directory_rejects_folder_with_recent_content() {
        let temp = tempfile::tempdir().expect("tempdir");
        let dir = temp.path().join("mixed_folder");
        std::fs::create_dir(&dir).expect("mkdir");
        let old = dir.join("old.bin");
        std::fs::write(&old, b"old").expect("write old");
        backdate_file(&old, 48);
        std::fs::write(dir.join("fresh.bin"), b"fresh").expect("write fresh");

        let job = make_job(day_cutoff(), cancel_flag(), Vec::new());
        assert!(
            measure_directory(&dir, &job, job.cutoff_unix, 0).is_none(),
            "folder containing a recent file must not be measurable as a candidate"
        );
    }

    #[test]
    fn measure_directory_sums_old_content() {
        let temp = tempfile::tempdir().expect("tempdir");
        let dir = temp.path().join("old_folder");
        std::fs::create_dir(&dir).expect("mkdir");
        let a = dir.join("a.bin");
        std::fs::write(&a, vec![0_u8; 10]).expect("write a");
        backdate_file(&a, 48);
        let b = dir.join("b.bin");
        std::fs::write(&b, vec![0_u8; 5]).expect("write b");
        backdate_file(&b, 48);

        let job = make_job(day_cutoff(), cancel_flag(), Vec::new());
        let (size, newest) =
            measure_directory(&dir, &job, job.cutoff_unix, 0).expect("measurable");
        assert_eq!(size, 15);
        assert!(newest < job.cutoff_unix);
    }

    #[test]
    fn cancel_flag_stops_collection() {
        let temp = tempfile::tempdir().expect("tempdir");
        for index in 0..50 {
            std::fs::write(temp.path().join(format!("file{index}.tmp")), b"x")
                .expect("write");
        }
        let cancel = Arc::new(AtomicBool::new(true));
        let job = make_job(
            u64::MAX,
            cancel,
            vec![category_for(temp.path().to_path_buf())],
        );
        let candidates = scan_all(&job).expect("scan should not fail");
        assert!(candidates.is_empty(), "cancelled scan collects nothing");
    }

    #[test]
    fn entries_outside_declared_root_are_never_candidates() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join("inside_parent.txt"), b"x").expect("write");

        // Root points at a non-existent subfolder; the real file lives in the
        // parent, outside the declared root.
        let job = make_job(
            u64::MAX,
            cancel_flag(),
            vec![category_for(temp.path().join("nested_root"))],
        );
        let candidates = scan_all(&job).expect("scan should not fail");
        assert!(candidates.is_empty());
    }

    #[test]
    fn pattern_roots_accept_only_matching_files() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join("thumbcache_1.db"), b"x").expect("write");
        std::fs::write(temp.path().join("iconcache_2.db"), b"x").expect("write");
        std::fs::write(temp.path().join("important_data.txt"), b"x").expect("write");
        std::fs::create_dir(temp.path().join("subdir")).expect("mkdir");

        let job = make_job(
            u64::MAX,
            cancel_flag(),
            vec![CleanupCategory {
                id: CleanupCategoryId::ExplorerThumbnailCache,
                roots: vec![temp.path().to_path_buf()],
                file_patterns: Some(vec!["thumbcache_", "iconcache_"]),
                requires_admin: false,
            }],
        );
        let candidates = scan_all(&job).expect("scan should not fail");

        assert_eq!(candidates.len(), 2);
        assert!(candidates
            .iter()
            .all(|candidate| candidate.path.extension().and_then(|extension| extension.to_str())
                == Some("db")));
    }

    #[test]
    fn run_cleanup_scan_sends_completed_message() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join("a.txt"), b"x").expect("write");

        let (sender, receiver) = channel();
        let job = CleanupScanJob {
            categories: vec![category_for(temp.path().to_path_buf())],
            cutoff_unix: u64::MAX,
            cancel: cancel_flag(),
            progress: Arc::new(std::sync::Mutex::new(CleanupScanProgress::default())),
            sender,
            generation: 7,
        };
        run_cleanup_scan(job);

        match receiver.recv() {
            Ok(CleanupScanMessage::Completed {
                generation,
                candidates,
            }) => {
                assert_eq!(generation, 7);
                assert_eq!(candidates.len(), 1);
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }
}
