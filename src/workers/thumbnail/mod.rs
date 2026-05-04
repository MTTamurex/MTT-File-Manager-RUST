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
pub mod progress;
pub mod queue;
pub mod types;
pub mod worker;

pub use progress::{
    begin_bulk_thumbnail_progress, clear_bulk_thumbnail_progress,
    new_shared_bulk_thumbnail_progress, set_bulk_thumbnail_current_file, BulkThumbnailProgress,
    SharedBulkThumbnailProgress,
};
pub use queue::PriorityThumbnailQueue;
pub use types::{ThumbnailPriority, ThumbnailRequest};
pub use worker::spawn_thumbnail_workers;

use lru::LruCache;
use parking_lot::Mutex;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::infrastructure::io_priority::IOPriority;

// --- Capacity constants for failure caches ---
const FAILED_PATHS_CAP: usize = 2048;
const FAILURE_BACKOFF_CAP: usize = 4096;
const ACTIVE_WRITE_BLOCK_MS: u64 = 2500;

/// Max entries tracked in the deferred-retry registry.
const UNSAFE_REGISTRY_CAP: usize = 2048;
/// Drop entries that have been waiting longer than this without becoming safe.
const UNSAFE_REGISTRY_MAX_AGE: Duration = Duration::from_secs(30 * 60);

/// Global cache of paths that failed thumbnail extraction (shared across workers)
/// Uses LRU eviction so oldest failures are dropped instead of clearing everything.
static FAILED_PATHS_CACHE: std::sync::OnceLock<Mutex<LruCache<PathBuf, ()>>> =
    std::sync::OnceLock::new();

fn get_failed_paths() -> &'static Mutex<LruCache<PathBuf, ()>> {
    FAILED_PATHS_CACHE
        .get_or_init(|| Mutex::new(LruCache::new(NonZeroUsize::new(FAILED_PATHS_CAP).unwrap())))
}

#[derive(Clone, Copy)]
struct FailureBackoffState {
    attempts: u8,
    retry_after: Instant,
}

static FAILURE_BACKOFF_CACHE: std::sync::OnceLock<Mutex<LruCache<PathBuf, FailureBackoffState>>> =
    std::sync::OnceLock::new();

fn get_failure_backoff() -> &'static Mutex<LruCache<PathBuf, FailureBackoffState>> {
    FAILURE_BACKOFF_CACHE.get_or_init(|| {
        Mutex::new(LruCache::new(
            NonZeroUsize::new(FAILURE_BACKOFF_CAP).unwrap(),
        ))
    })
}

fn compute_backoff(attempts: u8) -> Duration {
    // Exponential backoff capped to keep recovery responsive after transient spikes.
    let shift = attempts.saturating_sub(1).min(6) as u32;
    let ms = 400_u64
        .saturating_mul(2_u64.saturating_pow(shift))
        .min(20_000);
    Duration::from_millis(ms)
}

/// Check if a path has previously failed extraction (permanent or under backoff).
pub fn is_known_failure(path: &PathBuf) -> bool {
    if is_permanent_failure(path) {
        return true;
    }

    let map = get_failure_backoff().lock();
    map.peek(path)
        .is_some_and(|state| Instant::now() < state.retry_after)
}

/// Check if a path is permanently failed.
pub fn is_permanent_failure(path: &PathBuf) -> bool {
    get_failed_paths().lock().contains(path)
}

/// Mark a path as failed (won't retry until app restart).
/// LRU eviction ensures oldest entries are dropped transparently.
pub fn mark_as_failed(path: PathBuf) {
    get_failed_paths().lock().put(path, ());
}

/// Register a transient failure using exponential backoff.
/// Requests can retry automatically after the cooldown expires.
pub fn mark_as_transient_failure(path: PathBuf) {
    const MAX_TRANSIENT_ATTEMPTS: u8 = 6;

    let mut map = get_failure_backoff().lock();
    let attempts = map
        .peek(&path)
        .map_or(1, |state| state.attempts.saturating_add(1));

    if attempts >= MAX_TRANSIENT_ATTEMPTS {
        map.pop(&path);
        drop(map);
        mark_as_failed(path);
        return;
    }

    let retry_after = Instant::now() + compute_backoff(attempts);
    map.put(
        path,
        FailureBackoffState {
            attempts,
            retry_after,
        },
    );
}

/// Register a short-lived block when the file is actively being written
/// (download/encode in progress). This should never escalate to permanent
/// failure because the condition is expected to recover shortly.
pub fn mark_as_temporarily_blocked(path: PathBuf) {
    let retry_after = Instant::now() + Duration::from_millis(ACTIVE_WRITE_BLOCK_MS);
    get_failure_backoff().lock().put(
        path,
        FailureBackoffState {
            attempts: 0,
            retry_after,
        },
    );
}

/// Clear transient failure status after a successful load.
pub fn clear_transient_failure(path: &PathBuf) {
    get_failure_backoff().lock().pop(path);
}

/// Clear failure status for a specific path (allows retry)
/// Used when manually refreshing a thumbnail after file changes
pub fn clear_failure_cache(path: &PathBuf) {
    get_failed_paths().lock().pop(path);
    get_failure_backoff().lock().pop(path);
}

/// Clear all failure status (allows retry for everything)
/// Used when manually refreshing the entire folder (F5)
pub fn clear_all_failures() {
    get_failed_paths().lock().clear();
    get_failure_backoff().lock().clear();
}

// ---------------------------------------------------------------------------
// Deferred-retry registry for UnsafeToRead files
//
// When thumbnail extraction is deferred because the file is being written
// (e.g. an active qBittorrent download), we record the request here.
// A dedicated retry thread (`spawn_deferred_retry_thread`) polls this registry
// every ~1 s and re-injects requests into the queue as soon as the file
// becomes safe to read.
// ---------------------------------------------------------------------------

/// Metadata stored per deferred path so the retry thread can recreate the request.
#[derive(Clone)]
pub struct DeferredThumbnailEntry {
    pub req_size: u32,
    pub req_priority: IOPriority,
    pub req_modified: u64,
    pub req_generation: usize,
    pub inserted_at: Instant,
}

static UNSAFE_REGISTRY: std::sync::OnceLock<
    Mutex<LruCache<PathBuf, DeferredThumbnailEntry>>,
> = std::sync::OnceLock::new();

fn get_unsafe_registry() -> &'static Mutex<LruCache<PathBuf, DeferredThumbnailEntry>> {
    UNSAFE_REGISTRY.get_or_init(|| {
        Mutex::new(LruCache::new(
            NonZeroUsize::new(UNSAFE_REGISTRY_CAP).unwrap(),
        ))
    })
}

/// Register a file as deferred so the retry thread re-queues it once it is safe.
pub fn defer_unsafe_thumbnail(path: PathBuf, entry: DeferredThumbnailEntry) {
    get_unsafe_registry().lock().put(path, entry);
}

/// Drain all entries from the registry, returning them as a `Vec`.
/// Caller is responsible for re-inserting any that are still not safe.
pub fn drain_unsafe_registry() -> Vec<(PathBuf, DeferredThumbnailEntry)> {
    let mut cache = get_unsafe_registry().lock();
    let entries: Vec<(PathBuf, DeferredThumbnailEntry)> = cache
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    cache.clear();
    entries
}

/// Remove a single path from the deferred registry (e.g. on successful extraction).
pub fn remove_from_unsafe_registry(path: &PathBuf) {
    get_unsafe_registry().lock().pop(path);
}

/// Returns the number of entries currently in the deferred registry.
#[allow(dead_code)]
pub fn unsafe_registry_len() -> usize {
    get_unsafe_registry().lock().len()
}

/// Expiry helper: is this entry older than `UNSAFE_REGISTRY_MAX_AGE`?
pub fn deferred_entry_expired(entry: &DeferredThumbnailEntry) -> bool {
    entry.inserted_at.elapsed() >= UNSAFE_REGISTRY_MAX_AGE
}
