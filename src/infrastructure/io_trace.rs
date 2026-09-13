//! Lightweight counters attributing background disk I/O to its source.
//!
//! Scrolling a folder with thumbnails can trigger reads on the source drive
//! from several independent paths: consistency probes, folder-cover scans,
//! thumbnail extraction from the original file, drive-info refreshes and
//! folder-size walks. These counters make it possible to tell from the
//! diagnostic log which of them is actually hammering the disk.
//!
//! Counters are monotonic within the session; [`take_snapshot`] swaps them
//! back to zero so the periodic diagnostic report shows per-interval activity.

use std::sync::atomic::{AtomicU64, Ordering};

static CONSISTENCY_PROBES: AtomicU64 = AtomicU64::new(0);
static PROBE_COVER_SCANS: AtomicU64 = AtomicU64::new(0);
static PROBE_MS_TOTAL: AtomicU64 = AtomicU64::new(0);
static PROBE_MS_MAX: AtomicU64 = AtomicU64::new(0);
static COVER_SCAN_MS_MAX: AtomicU64 = AtomicU64::new(0);
static SOURCE_EXTRACTS: AtomicU64 = AtomicU64::new(0);
static TS_SNIFF_SKIPPED: AtomicU64 = AtomicU64::new(0);
static DRIVE_INFO_MS_MAX: AtomicU64 = AtomicU64::new(0);
static FOLDER_SIZE_WALKS: AtomicU64 = AtomicU64::new(0);
static FOLDER_SIZE_WALK_MS_MAX: AtomicU64 = AtomicU64::new(0);
static TAG_VALIDATION_PATHS: AtomicU64 = AtomicU64::new(0);
static TAG_VALIDATION_MS_MAX: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IoTraceSnapshot {
    pub consistency_probes: u64,
    pub probe_cover_scans: u64,
    pub probe_ms_total: u64,
    pub probe_ms_max: u64,
    pub cover_scan_ms_max: u64,
    pub source_extracts: u64,
    pub ts_sniff_skipped_by_cache: u64,
    pub drive_info_ms_max: u64,
    pub folder_size_walks: u64,
    pub folder_size_walk_ms_max: u64,
    pub tag_validation_paths: u64,
    pub tag_validation_ms_max: u64,
}

#[inline]
fn record_max(slot: &AtomicU64, value: u64) {
    slot.fetch_max(value, Ordering::Relaxed);
}

/// A directory consistency probe completed: `ms` is the total probe duration
/// and `cover_scans` counts the per-folder cover re-scans it performed.
pub fn record_consistency_probe(ms: u64, cover_scans: u64) {
    CONSISTENCY_PROBES.fetch_add(1, Ordering::Relaxed);
    PROBE_COVER_SCANS.fetch_add(cover_scans, Ordering::Relaxed);
    PROBE_MS_TOTAL.fetch_add(ms, Ordering::Relaxed);
    record_max(&PROBE_MS_MAX, ms);
}

/// A folder-cover discovery scan (`find_folder_preview_item`) took `ms`.
pub fn record_cover_scan(ms: u64) {
    record_max(&COVER_SCAN_MS_MAX, ms);
}

/// A thumbnail was extracted from the original source file (cache miss).
pub fn record_source_extract() {
    SOURCE_EXTRACTS.fetch_add(1, Ordering::Relaxed);
}

/// A `.ts` sync-byte sniff was avoided because the DB cache answered first.
pub fn record_ts_sniff_skipped_by_cache() {
    TS_SNIFF_SKIPPED.fetch_add(1, Ordering::Relaxed);
}

/// A drive-info query (volume info + hardware fields) took `ms`.
pub fn record_drive_info_query(ms: u64) {
    record_max(&DRIVE_INFO_MS_MAX, ms);
}

/// A recursive folder-size walk completed in `ms`.
pub fn record_folder_size_walk(ms: u64) {
    FOLDER_SIZE_WALKS.fetch_add(1, Ordering::Relaxed);
    record_max(&FOLDER_SIZE_WALK_MS_MAX, ms);
}

/// A tag-view existence sweep validated `paths` entries in `ms`. Each entry is
/// a `GetFileAttributesW` on the source drive, so on HDD/virtual drives this
/// must stay chunked and throttled.
pub fn record_tag_validation(paths: u64, ms: u64) {
    TAG_VALIDATION_PATHS.fetch_add(paths, Ordering::Relaxed);
    record_max(&TAG_VALIDATION_MS_MAX, ms);
}

/// Swaps all counters to zero and returns their previous values.
pub fn take_snapshot() -> IoTraceSnapshot {
    IoTraceSnapshot {
        consistency_probes: CONSISTENCY_PROBES.swap(0, Ordering::Relaxed),
        probe_cover_scans: PROBE_COVER_SCANS.swap(0, Ordering::Relaxed),
        probe_ms_total: PROBE_MS_TOTAL.swap(0, Ordering::Relaxed),
        probe_ms_max: PROBE_MS_MAX.swap(0, Ordering::Relaxed),
        cover_scan_ms_max: COVER_SCAN_MS_MAX.swap(0, Ordering::Relaxed),
        source_extracts: SOURCE_EXTRACTS.swap(0, Ordering::Relaxed),
        ts_sniff_skipped_by_cache: TS_SNIFF_SKIPPED.swap(0, Ordering::Relaxed),
        drive_info_ms_max: DRIVE_INFO_MS_MAX.swap(0, Ordering::Relaxed),
        folder_size_walks: FOLDER_SIZE_WALKS.swap(0, Ordering::Relaxed),
        folder_size_walk_ms_max: FOLDER_SIZE_WALK_MS_MAX.swap(0, Ordering::Relaxed),
        tag_validation_paths: TAG_VALIDATION_PATHS.swap(0, Ordering::Relaxed),
        tag_validation_ms_max: TAG_VALIDATION_MS_MAX.swap(0, Ordering::Relaxed),
    }
}
