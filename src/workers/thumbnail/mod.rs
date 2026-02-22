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
use std::time::{Duration, Instant};

/// Global cache of paths that failed thumbnail extraction (shared across workers)
/// Prevents re-attempting extraction on files that consistently fail (e.g., corrupt files)
static FAILED_PATHS_CACHE: std::sync::OnceLock<Mutex<FxHashSet<PathBuf>>> =
    std::sync::OnceLock::new();

fn get_failed_paths() -> &'static Mutex<FxHashSet<PathBuf>> {
    FAILED_PATHS_CACHE.get_or_init(|| Mutex::new(FxHashSet::default()))
}

#[derive(Clone, Copy)]
struct FailureBackoffState {
    attempts: u8,
    retry_after: Instant,
}

static FAILURE_BACKOFF_CACHE: std::sync::OnceLock<Mutex<rustc_hash::FxHashMap<PathBuf, FailureBackoffState>>> =
    std::sync::OnceLock::new();

fn get_failure_backoff() -> &'static Mutex<rustc_hash::FxHashMap<PathBuf, FailureBackoffState>> {
    FAILURE_BACKOFF_CACHE.get_or_init(|| Mutex::new(rustc_hash::FxHashMap::default()))
}

fn compute_backoff(attempts: u8) -> Duration {
    // Exponential backoff capped to keep recovery responsive after transient spikes.
    let shift = attempts.saturating_sub(1).min(6) as u32;
    let ms = 400_u64.saturating_mul(2_u64.saturating_pow(shift)).min(20_000);
    Duration::from_millis(ms)
}

/// Check if a path has previously failed extraction (permanent or under backoff).
pub fn is_known_failure(path: &PathBuf) -> bool {
    if is_permanent_failure(path) {
        return true;
    }

    get_failure_backoff()
        .lock()
        .map(|map| {
            map.get(path)
                .is_some_and(|state| Instant::now() < state.retry_after)
        })
        .unwrap_or(false)
}

    /// Check if a path is permanently failed.
    pub fn is_permanent_failure(path: &PathBuf) -> bool {
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

/// Register a transient failure using exponential backoff.
/// Requests can retry automatically after the cooldown expires.
pub fn mark_as_transient_failure(path: PathBuf) {
    const MAX_TRANSIENT_ATTEMPTS: u8 = 6;

    if let Ok(mut map) = get_failure_backoff().lock() {
        if map.len() > 4096 {
            map.clear();
        }

        let attempts = map
            .get(&path)
            .map_or(1, |state| state.attempts.saturating_add(1));

        if attempts >= MAX_TRANSIENT_ATTEMPTS {
            map.remove(&path);
            drop(map);
            mark_as_failed(path);
            return;
        }

        let retry_after = Instant::now() + compute_backoff(attempts);
        map.insert(
            path,
            FailureBackoffState {
                attempts,
                retry_after,
            },
        );
    }
}

/// Clear transient failure status after a successful load.
pub fn clear_transient_failure(path: &PathBuf) {
    if let Ok(mut map) = get_failure_backoff().lock() {
        map.remove(path);
    }
}

/// Clear failure status for a specific path (allows retry)
/// Used when manually refreshing a thumbnail after file changes
pub fn clear_failure_cache(path: &PathBuf) {
    if let Ok(mut set) = get_failed_paths().lock() {
        set.remove(path);
    }
    if let Ok(mut map) = get_failure_backoff().lock() {
        map.remove(path);
    }
}

/// Clear all failure status (allows retry for everything)
/// Used when manually refreshing the entire folder (F5)
pub fn clear_all_failures() {
    if let Ok(mut set) = get_failed_paths().lock() {
        set.clear();
    }
    if let Ok(mut map) = get_failure_backoff().lock() {
        map.clear();
    }
}
