//! Thumbnail worker system
//!
//! A multi-threaded, priority-based thumbnail extraction system with:
//! - 5-stage hybrid extraction pipeline
//! - SSD/HDD-aware queue optimization
//! - RAM-limiting semaphore for concurrent decodes
//! - Disk cache integration (SQLite)
//! - Failure tracking to avoid repeated attempts on broken files

pub mod extraction;
pub mod processing;
pub mod queue;
pub mod types;
pub mod worker;

pub use queue::PriorityThumbnailQueue;
pub use types::{ThumbnailPriority, ThumbnailRequest};
pub use worker::spawn_thumbnail_workers;

use rustc_hash::FxHashSet;
use std::path::PathBuf;
use std::sync::Mutex;

/// Global cache of paths that failed thumbnail extraction (shared across workers)
/// Prevents re-attempting extraction on files that consistently fail (e.g., corrupt files)
static FAILED_PATHS_CACHE: std::sync::OnceLock<Mutex<FxHashSet<PathBuf>>> =
    std::sync::OnceLock::new();

fn get_failed_paths() -> &'static Mutex<FxHashSet<PathBuf>> {
    FAILED_PATHS_CACHE.get_or_init(|| Mutex::new(FxHashSet::default()))
}

/// Check if a path has previously failed extraction
pub fn is_known_failure(path: &PathBuf) -> bool {
    get_failed_paths()
        .lock()
        .map(|set| set.contains(path))
        .unwrap_or(false)
}

/// Mark a path as failed (won't retry until app restart)
pub fn mark_as_failed(path: PathBuf) {
    if let Ok(mut set) = get_failed_paths().lock() {
        // Limit cache size to prevent memory issues (keep last 1000 failures)
        if set.len() > 1000 {
            set.clear();
        }
        set.insert(path);
    }
}

/// Clear failure status for a specific path (allows retry)
/// Used when manually refreshing a thumbnail after file changes
pub fn clear_failure_cache(path: &PathBuf) {
    if let Ok(mut set) = get_failed_paths().lock() {
        set.remove(path);
    }
}

/// Clear all failure status (allows retry for everything)
/// Used when manually refreshing the entire folder (F5)
pub fn clear_all_failures() {
    if let Ok(mut set) = get_failed_paths().lock() {
        set.clear();
    }
}
