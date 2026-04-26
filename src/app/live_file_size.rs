use crate::ui::cache::FxHashSet;
use lru::LruCache;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

pub type LiveFileSizeRequest = (PathBuf, u64);
pub type LiveFileSizeResponse = (PathBuf, u64, Option<u64>);

/// Match the previous UX contract: only probe recently-modified files.
pub const LIVE_SIZE_PROBE_MAX_AGE_SECS: u64 = 300;

pub fn should_probe_live_file_size(path: &Path, modified_epoch: u64) -> bool {
    if modified_epoch > 0 {
        let now_epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if now_epoch.saturating_sub(modified_epoch) > LIVE_SIZE_PROBE_MAX_AGE_SECS {
            return false;
        }
    }

    if crate::infrastructure::onedrive::is_onedrive_path(path) {
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
    cache: &mut LruCache<PathBuf, (u64, u64)>,
    loading: &mut FxHashSet<PathBuf>,
    request_sender: &mpsc::Sender<LiveFileSizeRequest>,
) -> u64 {
    if let Some(&(cached_mtime, cached_size)) = cache.peek(path) {
        if cached_mtime == modified_epoch {
            return cached_size;
        }
    }

    if !should_probe_live_file_size(path, modified_epoch) {
        return fallback_size;
    }

    if !loading.contains(path) {
        let request_path = path.clone();
        if request_sender
            .send((request_path.clone(), modified_epoch))
            .is_ok()
        {
            loading.insert(request_path);
        }
    }

    fallback_size
}
