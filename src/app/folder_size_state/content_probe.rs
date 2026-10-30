use super::FolderSizeState;
use crate::domain::file_tag::normalize_tag_path_key;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Clone, Copy, Debug)]
pub struct DisplayFolderSize {
    pub size: u64,
    pub checked_at: Instant,
}

#[derive(Debug)]
pub struct PendingContentProbe {
    pub path: PathBuf,
    pub paths: Vec<PathBuf>,
    pub started: Instant,
}

impl FolderSizeState {
    pub fn prepare_content_probe(
        &self,
        current: &Path,
        now: Instant,
    ) -> Option<(PendingContentProbe, u32)> {
        if self.content_probe_pending.is_some() {
            return None;
        }
        let paths = self.content_probe_paths(current);
        if paths.is_empty() {
            return None;
        }
        // Keep watermarks with each bounded display entry, independent of focus
        // changes, watcher resets and unsuccessful service requests.
        let oldest = paths
            .iter()
            .filter_map(|path| self.display_size_cache.peek(path).map(|v| v.checked_at))
            .min()?;
        let threshold = now
            .saturating_duration_since(oldest)
            .as_secs()
            .saturating_add(2)
            .clamp(5, u32::MAX as u64) as u32;
        Some((
            PendingContentProbe {
                path: current.to_path_buf(),
                paths,
                started: now,
            },
            threshold,
        ))
    }

    pub fn finish_content_probe(&mut self, path: &Path, succeeded: bool) -> bool {
        if self
            .content_probe_pending
            .as_ref()
            .map(|p| p.path.as_path())
            != Some(path)
        {
            return false;
        }
        let pending = self.content_probe_pending.take().unwrap();
        if succeeded {
            for path in pending.paths {
                if let Some(value) = self.display_size_cache.peek_mut(&path) {
                    // Do not regress a newer completed measurement, or extend
                    // past the query's start while the worker was delayed.
                    value.checked_at = value.checked_at.max(pending.started);
                }
            }
        }
        true
    }

    /// Monitor only known displayed sizes in this listing (or its details panel).
    /// The display LRU bounds the query; no directory enumeration is needed.
    pub fn content_probe_paths(&self, current: &Path) -> Vec<PathBuf> {
        let key = normalize_tag_path_key(current);
        self.display_size_cache
            .iter()
            .filter_map(|(path, _)| {
                let in_listing = path
                    .parent()
                    .is_some_and(|parent| normalize_tag_path_key(parent) == key);
                (in_listing || normalize_tag_path_key(path) == key).then(|| path.clone())
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::folder_size_state::tests::test_state;
    use std::time::Duration;

    #[test]
    fn content_queries_are_bounded_to_confirmed_sizes_for_the_current_listing() {
        let mut state = test_state();
        for path in [
            "folder",
            "folder/child",
            "folder/child/nested",
            "other/child",
        ] {
            state.cache_completed_folder_size(PathBuf::from(path), 100);
        }
        state
            .batch_progress
            .insert(PathBuf::from("folder/partial"), 10);
        state.invalidate_calculated_folder_size(Path::new("folder/child"));
        let mut paths = state.content_probe_paths(Path::new("folder"));
        paths.sort();
        assert_eq!(
            paths,
            vec![PathBuf::from("folder"), PathBuf::from("folder/child")]
        );
        assert!(state.content_probe_paths(Path::new("unrelated")).is_empty());
    }

    #[test]
    fn navigation_does_not_queue_another_probe_before_the_old_one_is_acknowledged() {
        let mut state = test_state();
        let pending = PathBuf::from("folder");
        state.content_probe_pending = Some(PendingContentProbe {
            path: pending.clone(),
            paths: Vec::new(),
            started: std::time::Instant::now(),
        });
        state.cancel_batch();
        assert_eq!(
            state.content_probe_pending.as_ref().map(|p| &p.path),
            Some(&pending)
        );
    }

    #[test]
    fn query_size_never_exceeds_the_display_cache_capacity() {
        let mut state = test_state();
        for n in 0..100 {
            state.cache_completed_folder_size(PathBuf::from(format!("folder/child-{n}")), n);
        }
        assert_eq!(state.content_probe_paths(Path::new("folder")).len(), 8);
    }

    #[cfg(windows)]
    #[test]
    fn listing_matching_is_case_insensitive() {
        let mut state = test_state();
        state.cache_completed_folder_size(PathBuf::from(r"C:\Folder\Child"), 100);
        assert_eq!(state.content_probe_paths(Path::new(r"c:\folder")).len(), 1);
    }

    #[test]
    fn failed_queries_keep_the_original_watermark_across_focus_changes() {
        let mut state = test_state();
        let folder = Path::new("folder");
        let child = folder.join("child");
        let baseline = Instant::now();
        state.display_size_cache.put(
            child.clone(),
            DisplayFolderSize {
                size: 10,
                checked_at: baseline,
            },
        );
        let (pending, threshold) = state
            .prepare_content_probe(folder, baseline + Duration::from_secs(30))
            .unwrap();
        assert_eq!(threshold, 32);
        state.content_probe_pending = Some(pending);
        assert!(!state.finish_content_probe(Path::new("other"), true));
        assert!(state
            .prepare_content_probe(Path::new("other"), baseline + Duration::from_secs(31))
            .is_none());
        assert!(state.finish_content_probe(folder, false));
        assert_eq!(
            state.display_size_cache.peek(&child).unwrap().checked_at,
            baseline
        );
        let (_, threshold) = state
            .prepare_content_probe(folder, baseline + Duration::from_secs(60))
            .unwrap();
        assert_eq!(threshold, 62);
    }

    #[test]
    fn successful_queries_use_request_start_not_delayed_completion_time() {
        let mut state = test_state();
        let folder = Path::new("folder");
        let child = folder.join("child");
        state.cache_completed_folder_size(child.clone(), 10);
        let sent = Instant::now() + Duration::from_secs(30);
        let (pending, _) = state.prepare_content_probe(folder, sent).unwrap();
        state.content_probe_pending = Some(pending);
        assert!(state.finish_content_probe(folder, true));
        assert_eq!(
            state.display_size_cache.peek(&child).unwrap().checked_at,
            sent
        );
        let (_, threshold) = state
            .prepare_content_probe(folder, sent + Duration::from_secs(20))
            .unwrap();
        assert_eq!(threshold, 22);
    }

    #[test]
    fn delayed_ack_does_not_regress_a_newer_measurement_or_reinsert_evicted_paths() {
        let mut state = test_state();
        let folder = Path::new("folder");
        let child = folder.join("child");
        let sent = Instant::now();
        state.cache_completed_folder_size(child.clone(), 10);
        let (pending, _) = state.prepare_content_probe(folder, sent).unwrap();
        state.content_probe_pending = Some(pending);
        let newer = sent + Duration::from_secs(10);
        state
            .display_size_cache
            .peek_mut(&child)
            .unwrap()
            .checked_at = newer;
        assert!(state.finish_content_probe(folder, true));
        assert_eq!(
            state.display_size_cache.peek(&child).unwrap().checked_at,
            newer
        );
        let (pending, _) = state.prepare_content_probe(folder, newer).unwrap();
        state.content_probe_pending = Some(pending);
        state.display_size_cache.pop(&child);
        assert!(state.finish_content_probe(folder, true));
        assert!(state.display_size_cache.is_empty());
    }
}
