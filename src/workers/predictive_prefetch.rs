use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::infrastructure::directory_cache::DirectoryCache;
use crate::infrastructure::io_priority::{self, IOPriority};
use crate::infrastructure::ntfs_reader;

const MAX_PREFETCH_PER_CYCLE: usize = 5;
const MIN_PREFETCH_INTERVAL: Duration = Duration::from_millis(500);

pub enum PredictiveMessage {
    NavigatedTo(PathBuf),
    HistoryUpdated(Vec<PathBuf>),
    Shutdown,
}

#[derive(Debug)]
struct PrefetchPrediction {
    path: PathBuf,
    confidence: f32,
    reason: &'static str,
}

pub struct PredictivePrefetcher {
    current_path: Option<PathBuf>,
    history: VecDeque<PathBuf>,
    last_prefetch: Instant,
}

impl PredictivePrefetcher {
    pub fn new() -> Self {
        Self {
            current_path: None,
            history: VecDeque::with_capacity(10),
            last_prefetch: Instant::now(),
        }
    }

    /// Predict directories the user is likely to navigate to next.
    /// PERFORMANCE: Uses directory_cache data instead of std::fs::read_dir() to avoid HDD I/O.
    /// If a directory isn't cached, we skip that prediction category entirely (zero disk access).
    fn predict(&self, directory_cache: &DirectoryCache) -> Vec<PrefetchPrediction> {
        let mut predictions = Vec::new();

        let Some(current) = &self.current_path else {
            return predictions;
        };

        // Parent directory (high confidence - user often goes back)
        if let Some(parent) = current.parent() {
            predictions.push(PrefetchPrediction {
                path: parent.to_path_buf(),
                confidence: 0.9,
                reason: "parent_directory",
            });
        }

        // Sibling directories: use cached parent data instead of std::fs::read_dir(parent)
        if let Some(parent) = current.parent() {
            let parent_buf = parent.to_path_buf();
            if let Some(cached_entries) = directory_cache.get(&parent_buf) {
                for entry in cached_entries.iter().filter(|e| e.is_dir).take(5) {
                    if entry.path != *current {
                        predictions.push(PrefetchPrediction {
                            path: entry.path.clone(),
                            confidence: 0.5,
                            reason: "sibling_directory",
                        });
                    }
                }
            }
            // If parent isn't cached, skip siblings (no HDD I/O)
        }

        // Child subdirectories: use cached current data instead of std::fs::read_dir(current)
        if let Some(cached_entries) = directory_cache.get(current) {
            for entry in cached_entries.iter().filter(|e| e.is_dir).take(3) {
                predictions.push(PrefetchPrediction {
                    path: entry.path.clone(),
                    confidence: 0.6,
                    reason: "first_subdirectory",
                });
            }
        }
        // If current isn't cached, skip children (no HDD I/O)

        // History-based predictions (no I/O needed - just memory)
        for (i, hist_path) in self.history.iter().enumerate() {
            if hist_path != current {
                predictions.push(PrefetchPrediction {
                    path: hist_path.clone(),
                    confidence: 0.4 - (i as f32 * 0.05),
                    reason: "recent_history",
                });
            }
        }

        predictions.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        predictions.dedup_by(|a, b| a.path == b.path);
        predictions.truncate(MAX_PREFETCH_PER_CYCLE);

        predictions
    }

    pub fn on_navigate(&mut self, path: PathBuf) {
        if self.history.front() != Some(&path) {
            self.history.push_front(path.clone());
            if self.history.len() > 10 {
                self.history.pop_back();
            }
        }

        self.current_path = Some(path);
    }
}

impl Default for PredictivePrefetcher {
    fn default() -> Self {
        Self::new()
    }
}

pub fn spawn_predictive_prefetcher(
    receiver: Receiver<PredictiveMessage>,
    directory_cache: Arc<DirectoryCache>,
) {
    std::thread::spawn(move || {
        io_priority::set_thread_priority(IOPriority::Background);

        let mut prefetcher = PredictivePrefetcher::new();

        loop {
            // BLOCKING: Wait for message instead of polling
            match receiver.recv() {
                Ok(PredictiveMessage::NavigatedTo(path)) => {
                    // Skip predictive prefetch entirely for SSDs - raw disk speed is sufficient
                    if io_priority::is_ssd(&path) {
                        prefetcher.on_navigate(path);
                        continue;
                    }

                    prefetcher.on_navigate(path);

                    // Process predictions immediately after navigation
                    if prefetcher.last_prefetch.elapsed() >= MIN_PREFETCH_INTERVAL {
                        // PERFORMANCE: predict() uses directory_cache data, zero HDD I/O
                        let predictions = prefetcher.predict(&directory_cache);

                        for prediction in predictions {
                            // Already cached - skip
                            if directory_cache.get(&prediction.path).is_some() {
                                continue;
                            }

                            // Not cached - read from disk (this is the only HDD I/O, and only for
                            // directories that haven't been visited yet)
                            if let Some(entries) =
                                ntfs_reader::read_directory_fast(&prediction.path)
                            {
                                let file_entries: Vec<crate::domain::file_entry::FileEntry> =
                                    entries
                                        .into_iter()
                                        .filter(|e| {
                                            let is_hidden = (e.attributes & 0x02) != 0;
                                            let is_system = (e.attributes & 0x04) != 0;
                                            !is_hidden && !is_system && !e.name.starts_with('.')
                                        })
                                        .map(|e| crate::domain::file_entry::FileEntry {
                                            path: prediction.path.join(&e.name),
                                            name: e.name,
                                            is_dir: e.is_dir,
                                            size: if e.is_dir { 0 } else { e.size },
                                            modified: e.modified,
                                            folder_cover: None,
                                            drive_info: None,
                                            sync_status:
                                                crate::domain::file_entry::SyncStatus::None,
                                            deletion_date: None,
                                            recycle_original_path: None,
                                        })
                                        .collect();

                                directory_cache.put(prediction.path.clone(), file_entries);
                                eprintln!(
                                    "[PERF] Prefetch cached: {:?} ({})",
                                    prediction.path.file_name(),
                                    prediction.reason
                                );
                            }
                        }

                        prefetcher.last_prefetch = Instant::now();
                    }
                }
                Ok(PredictiveMessage::HistoryUpdated(history)) => {
                    prefetcher.history = history.into_iter().collect();
                }
                Ok(PredictiveMessage::Shutdown) => {
                    break;
                }
                Err(_) => {
                    break; // Channel closed
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::create_dir;
    use tempfile::tempdir;

    #[test]
    fn predict_includes_parent() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("root");
        let sub = root.join("sub");
        create_dir(&root).unwrap();
        create_dir(&sub).unwrap();

        let cache = DirectoryCache::new();
        let mut prefetcher = PredictivePrefetcher::new();
        prefetcher.on_navigate(sub.clone());
        let predictions = prefetcher.predict(&cache);

        assert!(predictions.iter().any(|p| p.path == root));
    }
}
