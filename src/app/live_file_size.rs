use lru::LruCache;
use rustc_hash::{FxHashMap, FxHashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Match the previous UX contract: only probe recently-modified files.
pub const LIVE_SIZE_PROBE_MAX_AGE_SECS: u64 = 300;
pub const LIVE_SIZE_REVALIDATE_INTERVAL: Duration = Duration::from_secs(1);
const LIVE_SIZE_STABLE_REVALIDATE_INTERVAL: Duration = Duration::from_secs(30);
const LIVE_SIZE_STABLE_OBSERVATIONS: u8 = 2;
const MAX_ACTIVE_LIVE_SIZE_REQUESTS: usize = 64;
static NEXT_LIVE_SIZE_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug)]
pub struct LiveFileSizeRequest {
    pub path: PathBuf,
    pub source_mtime_secs: u64,
    pub request_id: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservedLiveFileSize {
    pub size: u64,
    pub modified: Option<SystemTime>,
}

#[derive(Clone, Debug)]
pub struct LiveFileSizeResponse {
    pub path: PathBuf,
    pub source_mtime_secs: u64,
    pub request_id: u64,
    pub observed: Option<ObservedLiveFileSize>,
}

#[derive(Clone, Debug)]
pub struct LiveFileSizeCacheEntry {
    source_mtime_secs: u64,
    observed: Option<ObservedLiveFileSize>,
    checked_at: Instant,
    stable_observations: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActiveLiveFileSizeRequest {
    source_mtime_secs: u64,
    request_id: u64,
}

pub type LiveFileSizeCache = LruCache<PathBuf, LiveFileSizeCacheEntry>;

#[derive(Debug, Default)]
pub struct ActiveLiveFileSizeRequests {
    by_path: FxHashMap<PathBuf, ActiveLiveFileSizeRequest>,
    outstanding_ids: FxHashSet<u64>,
}

impl ActiveLiveFileSizeRequests {
    pub fn len(&self) -> usize {
        self.outstanding_ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.outstanding_ids.is_empty()
    }
}

pub fn cached_live_file_size(
    path: &PathBuf,
    modified_epoch: u64,
    cache: &LiveFileSizeCache,
) -> Option<u64> {
    cache.peek(path).and_then(|entry| {
        (entry.source_mtime_secs == modified_epoch)
            .then(|| entry.observed.as_ref().map(|observed| observed.size))
            .flatten()
    })
}

pub fn should_probe_live_file_size(path: &Path, modified_epoch: u64) -> bool {
    if modified_epoch > 0 {
        let now_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if now_epoch.saturating_sub(modified_epoch) > LIVE_SIZE_PROBE_MAX_AGE_SECS {
            return false;
        }
    }

    if crate::infrastructure::onedrive::is_cloud_sync_path(path) {
        return false;
    }

    if crate::infrastructure::io_priority::is_network_or_virtual(path) {
        return false;
    }

    true
}

pub fn resolve_cached_or_enqueue_live_file_size(
    path: &PathBuf,
    modified_epoch: u64,
    fallback_size: u64,
    cache: &mut LiveFileSizeCache,
    active_requests: &mut ActiveLiveFileSizeRequests,
    request_sender: &mpsc::Sender<LiveFileSizeRequest>,
) -> u64 {
    resolve_cached_or_enqueue_live_file_size_at(
        path,
        modified_epoch,
        fallback_size,
        cache,
        active_requests,
        request_sender,
        Instant::now(),
    )
}

fn resolve_cached_or_enqueue_live_file_size_at(
    path: &PathBuf,
    modified_epoch: u64,
    fallback_size: u64,
    cache: &mut LiveFileSizeCache,
    active_requests: &mut ActiveLiveFileSizeRequests,
    request_sender: &mpsc::Sender<LiveFileSizeRequest>,
    now: Instant,
) -> u64 {
    let matching_entry = cache
        .peek(path)
        .filter(|entry| entry.source_mtime_secs == modified_epoch);
    let resolved_size = matching_entry
        .and_then(|entry| entry.observed.as_ref().map(|observed| observed.size))
        .unwrap_or(fallback_size);

    if matching_entry.is_some_and(|entry| {
        let revalidate_interval = if entry.stable_observations >= LIVE_SIZE_STABLE_OBSERVATIONS {
            LIVE_SIZE_STABLE_REVALIDATE_INTERVAL
        } else {
            LIVE_SIZE_REVALIDATE_INTERVAL
        };
        now.saturating_duration_since(entry.checked_at) < revalidate_interval
    }) {
        return resolved_size;
    }

    let probe_mtime_secs = matching_entry
        .and_then(|entry| entry.observed.as_ref())
        .and_then(|observed| observed.modified)
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or(modified_epoch);
    if !should_probe_live_file_size(path, probe_mtime_secs) {
        return resolved_size;
    }

    if active_requests
        .by_path
        .get(path)
        .is_some_and(|request| request.source_mtime_secs == modified_epoch)
    {
        return resolved_size;
    }
    if active_requests.len() >= MAX_ACTIVE_LIVE_SIZE_REQUESTS {
        return resolved_size;
    }

    let request_id = NEXT_LIVE_SIZE_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let request = LiveFileSizeRequest {
        path: path.clone(),
        source_mtime_secs: modified_epoch,
        request_id,
    };
    if request_sender.send(request).is_ok() {
        active_requests.outstanding_ids.insert(request_id);
        active_requests.by_path.insert(
            path.clone(),
            ActiveLiveFileSizeRequest {
                source_mtime_secs: modified_epoch,
                request_id,
            },
        );
    }

    resolved_size
}

pub fn accept_live_file_size_response(
    response: LiveFileSizeResponse,
    cache: &mut LiveFileSizeCache,
    active_requests: &mut ActiveLiveFileSizeRequests,
    now: Instant,
) -> Option<Duration> {
    let expected = ActiveLiveFileSizeRequest {
        source_mtime_secs: response.source_mtime_secs,
        request_id: response.request_id,
    };
    if !active_requests.outstanding_ids.remove(&response.request_id)
        || active_requests.by_path.get(&response.path) != Some(&expected)
    {
        return None;
    }
    active_requests.by_path.remove(&response.path);

    let previous = cache
        .peek(&response.path)
        .filter(|entry| entry.source_mtime_secs == response.source_mtime_secs);
    let same_observation = previous.is_some_and(|entry| match &response.observed {
        Some(observed) => entry.observed.as_ref() == Some(observed),
        None => entry.observed.is_none(),
    });
    let stable_observations = if same_observation {
        previous
            .map(|entry| entry.stable_observations.saturating_add(1))
            .unwrap_or(0)
    } else {
        0
    };
    let observed = response
        .observed
        .or_else(|| previous.and_then(|entry| entry.observed.as_ref()).cloned());

    cache.put(
        response.path,
        LiveFileSizeCacheEntry {
            source_mtime_secs: response.source_mtime_secs,
            observed,
            checked_at: now,
            stable_observations,
        },
    );

    Some(if stable_observations >= LIVE_SIZE_STABLE_OBSERVATIONS {
        LIVE_SIZE_STABLE_REVALIDATE_INTERVAL
    } else {
        LIVE_SIZE_REVALIDATE_INTERVAL
    })
}

pub fn invalidate_live_file_size(
    path: &Path,
    cache: &mut LiveFileSizeCache,
    active_requests: &mut ActiveLiveFileSizeRequests,
) {
    cache.pop(path);
    active_requests.by_path.remove(path);
}

pub fn read_live_file_size(path: &Path) -> Option<ObservedLiveFileSize> {
    std::fs::metadata(path)
        .ok()
        .filter(|metadata| metadata.is_file())
        .map(|metadata| ObservedLiveFileSize {
            size: metadata.len(),
            modified: metadata.modified().ok(),
        })
}

#[cfg(test)]
#[path = "live_file_size_tests.rs"]
mod tests;
