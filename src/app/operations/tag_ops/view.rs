use super::*;
use crate::app::state::{FolderLoadError, ImageViewerApp};
use crate::domain::file_entry::FileEntry;
use crate::domain::special_paths::{tag_id_from_view_path, tag_view_path};
use crate::infrastructure::windows::RootAvailabilityCache;
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use std::time::{Duration, Instant};
use windows::core::PCWSTR;
use windows::Win32::Storage::FileSystem::{
    GetFileAttributesExW, GetFileExInfoStandard, INVALID_FILE_ATTRIBUTES, WIN32_FILE_ATTRIBUTE_DATA,
};

/// How long a path existence check is trusted before it is repeated.
///
/// Tag views are re-opened constantly while the user switches tags; re-sweeping
/// every path on each open issued 600-1500 `GetFileAttributesW` calls against
/// HDD/virtual drives every time. Reusing recent results makes a revisit nearly
/// free while the short TTL bounds how long a deleted file may linger in a tag
/// view.
const TAG_VALIDATION_TTL: Duration = Duration::from_secs(15);
/// Upper bound for the session-wide validation cache.
const TAG_VALIDATION_CACHE_LIMIT: usize = 20_000;

/// Returns `true` when `path` must be checked again, marking it as validated.
fn take_validation_slot(path: &std::path::Path) -> bool {
    static LAST_VALIDATED: std::sync::OnceLock<
        std::sync::Mutex<rustc_hash::FxHashMap<PathBuf, Instant>>,
    > = std::sync::OnceLock::new();

    let now = Instant::now();
    let cache =
        LAST_VALIDATED.get_or_init(|| std::sync::Mutex::new(rustc_hash::FxHashMap::default()));
    let mut guard = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    if !validation_is_due(guard.get(path).copied(), now) {
        return false;
    }
    if guard.len() >= TAG_VALIDATION_CACHE_LIMIT {
        prune_validation_cache(&mut guard, now, TAG_VALIDATION_CACHE_LIMIT);
    }
    guard.insert(path.to_path_buf(), now);
    true
}

/// Decision rule for [`take_validation_slot`], kept pure so it can be tested.
fn validation_is_due(last_validated: Option<Instant>, now: Instant) -> bool {
    match last_validated {
        Some(last) => now.saturating_duration_since(last) >= TAG_VALIDATION_TTL,
        None => true,
    }
}

/// Removes expired validation results and, when the cache is full, evicts the
/// oldest remaining results so the next insertion cannot exceed the limit.
fn prune_validation_cache(
    cache: &mut rustc_hash::FxHashMap<PathBuf, Instant>,
    now: Instant,
    max_entries: usize,
) {
    cache.retain(|_, validated_at| !validation_is_due(Some(*validated_at), now));

    if max_entries == 0 {
        cache.clear();
        return;
    }

    // Free a batch so a stream of new paths does not sort the entire cache
    // again on every insertion while all timestamps are still fresh.
    if cache.len() < max_entries {
        return;
    }
    let keep_entries = max_entries - (max_entries / 4).max(1);
    let remove_count = cache.len().saturating_sub(keep_entries);
    let mut oldest: Vec<(PathBuf, Instant)> = cache
        .iter()
        .map(|(path, validated_at)| (path.clone(), *validated_at))
        .collect();
    oldest.sort_unstable_by_key(|(_, validated_at)| *validated_at);
    for (path, _) in oldest.into_iter().take(remove_count) {
        cache.remove(&path);
    }
}

/// Re-attaches a persisted folder cover to a tag-view entry.
///
/// Tag items are rebuilt with `folder_cover: None` on every visit, and the
/// renderer treats a missing cover as "never scanned" — causing a fresh cover
/// discovery (directory enumeration) per visible folder on every revisit.
fn attach_folder_cover(
    entry: &mut FileEntry,
    covers: Option<&std::collections::HashMap<PathBuf, PathBuf>>,
) {
    if entry.is_dir {
        if let Some(cover) = covers.and_then(|covers| covers.get(&entry.path)) {
            entry.folder_cover = Some(cover.clone());
        }
    }
}

/// Convert a Windows FILETIME (100-nanosecond intervals since 1601-01-01) to
/// Unix seconds. Returns 0 for invalid/zero timestamps.
fn filetime_to_unix_secs(ft: &windows::Win32::Foundation::FILETIME) -> u64 {
    let ticks = ((ft.dwHighDateTime as u64) << 32) | (ft.dwLowDateTime as u64);
    if ticks > 116444736000000000 {
        (ticks - 116444736000000000) / 10_000_000
    } else {
        0
    }
}

fn tag_view_file_entry(path: PathBuf, show_hidden: bool) -> Option<FileEntry> {
    // Use GetFileAttributesExW instead of std::fs::metadata().
    // std::fs::metadata() calls CreateFileW + GetFileInformationByHandle + CloseHandle,
    // which opens a kernel handle per file (security check, share mode check, object
    // allocation/deallocation). On a cold NTFS cache this is 5-15ms per file on HDD.
    //
    // GetFileAttributesExW reads from the directory entry cache in a single kernel
    // call — no handle, no OneDrive file recall, returns in microseconds.
    // It returns WIN32_FILE_ATTRIBUTE_DATA with all fields we need: attributes,
    // file size, and creation/last-write timestamps.
    let path_wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut data = WIN32_FILE_ATTRIBUTE_DATA::default();
    let result = unsafe {
        GetFileAttributesExW(
            PCWSTR(path_wide.as_ptr()),
            GetFileExInfoStandard,
            &mut data as *mut _ as *mut std::ffi::c_void,
        )
    };
    if result.is_err() || data.dwFileAttributes == INVALID_FILE_ATTRIBUTES {
        return None;
    }

    let name = path.file_name()?.to_string_lossy().to_string();
    let is_archive = crate::domain::file_entry::is_archive_extension(&name);
    let attrs = data.dwFileAttributes;
    let is_real_dir = (attrs & 0x10) != 0; // FILE_ATTRIBUTE_DIRECTORY
    let is_dir = is_real_dir || is_archive;
    let is_hidden = (attrs & 0x2) != 0;
    if is_hidden && !show_hidden {
        return None;
    }

    let size = if is_real_dir && !is_archive {
        0
    } else {
        ((data.nFileSizeHigh as u64) << 32) | (data.nFileSizeLow as u64)
    };
    let modified = filetime_to_unix_secs(&data.ftLastWriteTime);
    let created = {
        let c = filetime_to_unix_secs(&data.ftCreationTime);
        if c > 0 {
            Some(c)
        } else {
            None
        }
    };
    let is_cloud = crate::infrastructure::onedrive::is_cloud_sync_path(&path);
    let sync_status = if is_cloud {
        crate::infrastructure::onedrive::get_sync_status(attrs, true)
    } else {
        crate::domain::file_entry::SyncStatus::None
    };

    Some(FileEntry {
        path,
        name,
        is_dir,
        size,
        modified,
        created,
        folder_cover: None,
        drive_info: None,
        sync_status,
        is_hidden,
        recycle_bin: None,
    })
}

impl ImageViewerApp {
    /// Returns the cached snapshot of tag IDs ordered by
    /// `(position, name case-insensitive)`. Cloning the `Arc` is O(1); callers
    /// resolve the current `FileTag` by ID via `self.tag_definitions`.
    pub fn sorted_tag_ids(&self) -> std::sync::Arc<[i64]> {
        self.sorted_tag_ids.clone()
    }

    /// Recomputes [`Self::sorted_tag_ids`] from the current definitions.
    /// Call after create, rename, delete, or position changes — never on
    /// assign/unassign or recolor.
    pub fn rebuild_sorted_tag_ids(&mut self) {
        self.sorted_tag_ids = super::build_sorted_tag_ids(&self.tag_definitions);
    }

    pub fn tag_view_display_name(&self, tag_id: i64) -> String {
        let name = self
            .tag_definitions
            .get(&tag_id)
            .map(|tag| tag.name.clone())
            .unwrap_or_else(|| tag_id.to_string());
        rust_i18n::t!("tags.filter_active", name = name).to_string()
    }

    pub fn tag_view_display_name_for_path(&self, path: &str) -> Option<String> {
        tag_id_from_view_path(path).map(|tag_id| self.tag_view_display_name(tag_id))
    }

    pub fn setup_tag_view(&mut self, tag_id: i64) {
        if !self.tag_definitions.contains_key(&tag_id) {
            self.active_tag_filter = None;
            self.navigate_to_computer();
            return;
        }

        let view_path = tag_view_path(tag_id);
        self.bump_folder_load_generation();
        self.invalidate_active_items_rebuild();

        self.navigation_state.current_path = view_path.clone();
        self.navigation_state.path_input = self.tag_view_display_name(tag_id);
        self.navigation_state.is_computer_view = false;
        self.navigation_state.is_recycle_bin_view = false;
        self.active_tag_filter = Some(tag_id);

        // Tag-view FileEntry cache intentionally does not persist folder_cover.
        // Reusing stale scan markers would prevent visible folders from
        // re-discovering covers after focus-restore texture flushes or tag switches.
        self.scanned_folders.clear();

        if self.uses_conservative_folder_preview_policy() {
            self.cache_manager.clear_folder_preview_inflight_state();
            self.pending_folder_preview_replace.clear();
            self.suppress_next_folder_preview_invalidation.clear();
        }

        self.apply_folder_lock_if_present();

        self.items = Arc::new(Vec::new());
        self.group_projection = Arc::new(Default::default());
        self.all_items_mut().clear();
        self.total_items = 0;
        self.is_loading_folder = true;
        self.folder_load_error = None;
        self.pending_all_items_clear = false;
        self.hold_visible_items_until_load_complete = false;
        self.pending_items_rebuild = false;
        self.pending_items_count = 0;
        self.loading_started_at = Instant::now();
        self.loaded_path = view_path;
        self.reset_selection_and_search();

        // The in-memory tag_assignments map is loaded at startup (init.rs)
        // and maintained by all tag mutation paths via sync_tag_assignments_normalized().
        // No need to reload from DB here — the filtered path list is queried
        // directly from SQLite in the background thread (see below).

        let my_gen = self.generation;
        let sender = self.file_entry_sender.clone();
        let ui_ctx = self.ui_ctx.clone();
        let show_hidden = self.show_hidden_files;
        let failure_path = PathBuf::from(&self.navigation_state.current_path);
        let app_state_db = self.app_state_db.clone();
        let tag_gc_sender = self.tag_assignment_gc_sender.clone();

        // Generation-aware cancellation: for the active panel, use the shared
        // current_generation atomic so the thread can detect navigation away.
        // For the inactive panel, use a local atomic that never changes
        // (bump_folder_load_generation skips current_generation for inactive
        // panel context, so using the shared atomic would cause immediate break).
        let gen_tracker: Arc<AtomicUsize> = if self.in_inactive_panel_context {
            Arc::new(AtomicUsize::new(self.generation))
        } else {
            self.current_generation.clone()
        };

        let spawn_result = std::thread::Builder::new()
            .name("tag-view-load".into())
            .spawn(move || {
                // Keep the first page small so the UI can paint before the
                // full tag assignment list has been materialized.
                const FIRST_PAGE_SIZE: i64 = 64;
                const PAGE_SIZE: i64 = 256;
                const CACHE_BATCH_SIZE: usize = 64;
                const FIRST_BATCH_SIZE: usize = 20;
                const BATCH_SIZE: usize = 100;

                let started = std::time::Instant::now();
                let mut first_batch_sent: Option<std::time::Instant> = None;
                let mut total_paths = 0usize;
                let mut total_cache_hits = 0usize;
                let mut total_cache_misses = 0usize;
                let mut page_size = FIRST_PAGE_SIZE;
                let mut last_path: Option<PathBuf> = None;
                let mut detected_is_ssd: Option<bool> = None;
                let mut cached_paths_to_validate: Vec<PathBuf> = Vec::new();
                let mut root_availability = RootAvailabilityCache::default();
                let mark_first = |first: &mut Option<std::time::Instant>| {
                    if first.is_none() {
                        *first = Some(std::time::Instant::now());
                    }
                };

                loop {
                    if gen_tracker.load(std::sync::atomic::Ordering::Relaxed) != my_gen {
                        break;
                    }

                    let current_page_size = page_size;
                    let paths = app_state_db.get_tag_assignment_paths_after(
                        tag_id,
                        last_path.as_deref(),
                        current_page_size,
                    );
                    if paths.is_empty() {
                        break;
                    }

                    total_paths = total_paths.saturating_add(paths.len());
                    last_path = paths.last().cloned();
                    page_size = PAGE_SIZE;

                    // Cache lookup for this page only. This bounds temporary
                    // RAM and lets the first cached rows paint immediately.
                    let cached: rustc_hash::FxHashMap<std::path::PathBuf, FileEntry> =
                        app_state_db.get_cached_file_entries(&paths);
                    total_cache_hits = total_cache_hits.saturating_add(cached.len());

                    // Re-attach persisted folder covers for this page. Tag items
                    // are rebuilt on every visit with `folder_cover: None`, and
                    // the renderer treats a missing cover as "never scanned" —
                    // so every revisit re-enumerated each visible folder
                    // (20-120 ms per folder on virtual drives, in bursts that
                    // stalled frames while scrolling). The cover worker persists
                    // discovered covers in `folder_covers`; reading them here
                    // keeps revisits off the source drive.
                    let cover_map: Option<
                        std::collections::HashMap<std::path::PathBuf, std::path::PathBuf>,
                    > = {
                        let covers = app_state_db.get_folder_covers(&paths);
                        (!covers.is_empty()).then_some(covers)
                    };
                    let attach_cover = |entry: &mut FileEntry| {
                        attach_folder_cover(entry, cover_map.as_ref());
                    };

                    if !cached.is_empty() {
                        let mut cached_batch = Vec::with_capacity(CACHE_BATCH_SIZE);
                        for path in &paths {
                            let Some(entry) = cached.get(path) else {
                                continue;
                            };
                            if !root_availability.is_root_accessible(&entry.path) {
                                continue;
                            }
                            cached_paths_to_validate.push(entry.path.clone());
                            if entry.is_hidden && !show_hidden {
                                continue;
                            }
                            let mut entry = entry.clone();
                            attach_cover(&mut entry);
                            cached_batch.push(entry);
                            if cached_batch.len() >= CACHE_BATCH_SIZE {
                                let _ = sender.send((my_gen, std::mem::take(&mut cached_batch)));
                                ui_ctx.request_repaint();
                                mark_first(&mut first_batch_sent);
                            }
                        }
                        if !cached_batch.is_empty() {
                            let _ = sender.send((my_gen, cached_batch));
                            ui_ctx.request_repaint();
                            mark_first(&mut first_batch_sent);
                        }
                    }

                    let misses: Vec<std::path::PathBuf> = paths
                        .iter()
                        .filter(|p| {
                            !cached.contains_key(*p)
                                && root_availability.is_root_accessible(p.as_path())
                        })
                        .cloned()
                        .collect();
                    total_cache_misses = total_cache_misses.saturating_add(misses.len());

                    let is_ssd = *detected_is_ssd.get_or_insert_with(|| {
                        misses
                            .first()
                            .or(paths.first())
                            .map(|p| crate::infrastructure::io_priority::is_ssd(p))
                            .unwrap_or(true)
                    });

                    let mut fresh_entries: Vec<FileEntry> = Vec::with_capacity(misses.len());
                    if is_ssd {
                        use rayon::prelude::*;
                        for chunk in misses.chunks(BATCH_SIZE) {
                            if gen_tracker.load(std::sync::atomic::Ordering::Relaxed) != my_gen {
                                break;
                            }
                            let mut chunk_entries: Vec<FileEntry> = chunk
                                .par_iter()
                                .filter_map(|p| tag_view_file_entry(p.clone(), show_hidden))
                                .collect();
                            for entry in chunk_entries.iter_mut() {
                                attach_cover(entry);
                            }
                            if !chunk_entries.is_empty() {
                                let _ = sender.send((my_gen, chunk_entries.clone()));
                                ui_ctx.request_repaint();
                                mark_first(&mut first_batch_sent);
                                fresh_entries.extend(chunk_entries);
                            }
                        }
                    } else {
                        let mut batch = Vec::with_capacity(FIRST_BATCH_SIZE);
                        let mut batch_index: usize = 0;
                        for path in &misses {
                            if gen_tracker.load(std::sync::atomic::Ordering::Relaxed) != my_gen {
                                break;
                            }
                            if let Some(mut entry) = tag_view_file_entry(path.clone(), show_hidden)
                            {
                                attach_cover(&mut entry);
                                batch.push(entry.clone());
                                fresh_entries.push(entry);
                                let current_batch_size = if batch_index < 2 {
                                    FIRST_BATCH_SIZE
                                } else {
                                    BATCH_SIZE
                                };
                                if batch.len() >= current_batch_size {
                                    let _ = sender.send((my_gen, std::mem::take(&mut batch)));
                                    ui_ctx.request_repaint();
                                    mark_first(&mut first_batch_sent);
                                    batch_index += 1;
                                    batch = Vec::with_capacity(if batch_index < 2 {
                                        FIRST_BATCH_SIZE
                                    } else {
                                        BATCH_SIZE
                                    });
                                }
                            }
                        }
                        if !batch.is_empty() {
                            let _ = sender.send((my_gen, batch));
                            ui_ctx.request_repaint();
                            mark_first(&mut first_batch_sent);
                        }
                    }

                    if !fresh_entries.is_empty() {
                        app_state_db.upsert_cached_file_entries(&fresh_entries);
                    }

                    if (paths.len() as i64) < current_page_size {
                        break;
                    }
                }

                // Always send end-of-load sentinel, even after generation-break.
                // The consumer discards batches from old generations, so this is safe.
                let _ = sender.send((my_gen, Vec::new()));
                ui_ctx.request_repaint();

                let time_to_first_ms = first_batch_sent
                    .map(|t| t.saturating_duration_since(started).as_millis() as i64)
                    .unwrap_or(-1);
                let total_ms = started.elapsed().as_millis() as i64;
                log::info!(
                    "[TAGS] setup_tag_view(tag_id={}) total_paths={} cache_hits={} \
                     cache_misses={} is_ssd={} time_to_first_batch_ms={} total_ms={}",
                    tag_id,
                    total_paths,
                    total_cache_hits,
                    total_cache_misses,
                    detected_is_ssd.unwrap_or(true),
                    time_to_first_ms,
                    total_ms
                );

                if !cached_paths_to_validate.is_empty() {
                    // Existence sweep: one GetFileAttributesW per cached path.
                    // On HDD/virtual drives (Cryptomator) a full-page sweep of
                    // 600-1500 paths used to hit the drive in a single burst
                    // right after opening a tag view. Sweep in small chunks with
                    // a short pause between them so the metadata I/O does not
                    // compete with thumbnail/browsing reads, and abort as soon
                    // as the user navigates away.
                    const VALIDATION_CHUNK_PATHS: usize = 64;
                    // HDD/virtual drives pay a seek per path (Cryptomator also
                    // decrypts), so pause longer between chunks there.
                    let validation_pause_ms = if detected_is_ssd.unwrap_or(true) {
                        1
                    } else {
                        5
                    };
                    let total_to_validate = cached_paths_to_validate.len();
                    let validation_start = std::time::Instant::now();
                    let mut checked_paths = 0usize;
                    let mut missing_candidates: Vec<PathBuf> = Vec::new();
                    for chunk in cached_paths_to_validate.chunks(VALIDATION_CHUNK_PATHS) {
                        if gen_tracker.load(std::sync::atomic::Ordering::Relaxed) != my_gen {
                            break;
                        }
                        let mut checked_in_chunk = 0usize;
                        for path in chunk {
                            if !take_validation_slot(path) {
                                continue;
                            }
                            checked_in_chunk += 1;
                            checked_paths += 1;
                            if !crate::infrastructure::onedrive::fast_path_exists(path) {
                                missing_candidates.push(path.clone());
                            }
                        }
                        if checked_in_chunk > 0 {
                            std::thread::sleep(std::time::Duration::from_millis(
                                validation_pause_ms,
                            ));
                        }
                    }
                    let validation_ms = validation_start.elapsed().as_millis() as u64;
                    crate::infrastructure::io_trace::record_tag_validation(
                        checked_paths as u64,
                        validation_ms,
                    );
                    if checked_paths > 0 && validation_ms >= 50 {
                        log::info!(
                            "[TAGS] path validation swept={} checked={} missing={} took_ms={}",
                            total_to_validate,
                            checked_paths,
                            missing_candidates.len(),
                            validation_ms
                        );
                    }
                    if !missing_candidates.is_empty() {
                        let _ = tag_gc_sender.send(TagPathUpdate::HideFromViews {
                            generation: my_gen,
                            paths: missing_candidates,
                        });
                    }
                }
            });

        if let Err(error) = spawn_result {
            let message = format!("Failed to spawn tag view loader: {}", error);
            log::error!("[TAGS] {}", message);
            self.is_loading_folder = false;
            self.folder_load_error = Some(FolderLoadError::other(
                failure_path.clone(),
                message.clone(),
            ));
            let _ = self
                .folder_load_failure_sender
                .send((my_gen, FolderLoadError::other(failure_path, message)));
            self.ui_ctx.request_repaint();
        }
    }

    pub fn reload_visible_tag_views(&mut self) -> bool {
        let active_tag_id = tag_id_from_view_path(&self.navigation_state.current_path);
        let inactive_tag_id = self
            .dual_panel_inactive_state
            .as_ref()
            .and_then(|snapshot| tag_id_from_view_path(&snapshot.path));

        if active_tag_id.is_none() && inactive_tag_id.is_none() {
            return false;
        }

        if let Some(tag_id) = active_tag_id {
            self.setup_tag_view(tag_id);
        }

        if let Some(tag_id) = inactive_tag_id {
            self.with_inactive_panel(|app| {
                app.setup_tag_view(tag_id);
            });
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::file_entry::SyncStatus;

    fn entry(path: &str, is_dir: bool) -> FileEntry {
        FileEntry {
            path: PathBuf::from(path),
            name: String::new(),
            is_dir,
            size: 0,
            modified: 0,
            created: None,
            folder_cover: None,
            drive_info: None,
            sync_status: SyncStatus::None,
            is_hidden: false,
            recycle_bin: None,
        }
    }

    #[test]
    fn validation_ttl_allows_first_check_and_blocks_recent_repeats() {
        let now = Instant::now();
        assert!(validation_is_due(None, now));
        assert!(!validation_is_due(
            Some(now),
            now + TAG_VALIDATION_TTL - Duration::from_secs(1)
        ));
        assert!(validation_is_due(Some(now), now + TAG_VALIDATION_TTL));
    }

    #[test]
    fn validation_cache_prunes_expired_entries() {
        let now = Instant::now();
        let expired = PathBuf::from("expired");
        let recent = PathBuf::from("recent");
        let mut cache = rustc_hash::FxHashMap::default();
        cache.insert(expired.clone(), now - TAG_VALIDATION_TTL);
        cache.insert(recent.clone(), now);

        prune_validation_cache(&mut cache, now, 3);

        assert!(!cache.contains_key(&expired));
        assert!(cache.contains_key(&recent));
    }

    #[test]
    fn validation_cache_evicts_oldest_entry_before_insertion() {
        let now = Instant::now();
        let oldest = PathBuf::from("oldest");
        let mut cache = rustc_hash::FxHashMap::default();
        cache.insert(oldest.clone(), now - Duration::from_secs(2));
        cache.insert(PathBuf::from("newer-a"), now - Duration::from_secs(1));
        cache.insert(PathBuf::from("newer-b"), now);

        prune_validation_cache(&mut cache, now, 3);
        cache.insert(PathBuf::from("incoming"), now);

        assert_eq!(cache.len(), 3);
        assert!(!cache.contains_key(&oldest));
    }

    #[test]
    fn validation_slots_are_consumed_once_per_ttl_window() {
        let path = std::path::PathBuf::from(r"Z:\Library\_validation_slot_test\file.mkv");
        assert!(take_validation_slot(&path));
        assert!(!take_validation_slot(&path));
    }

    #[test]
    fn full_validation_cache_leaves_room_for_a_batch() {
        let now = Instant::now();
        let mut cache = rustc_hash::FxHashMap::default();
        for index in 0..20 {
            cache.insert(PathBuf::from(format!("file-{index}")), now);
        }
        prune_validation_cache(&mut cache, now, 20);
        assert_eq!(cache.len(), 15);
        for index in 20..25 {
            cache.insert(PathBuf::from(format!("file-{index}")), now);
        }
        assert_eq!(cache.len(), 20);
    }

    #[test]
    fn folder_cover_attach_only_applies_to_directories_with_a_known_cover() {
        const FOLDER: &str = r"Z:\Library\Show Name";
        const COVER: &str = r"Z:\Library\Show Name\s01e01.mkv";
        let covers =
            std::collections::HashMap::from([(PathBuf::from(FOLDER), PathBuf::from(COVER))]);

        let mut directory = entry(FOLDER, true);
        attach_folder_cover(&mut directory, Some(&covers));
        assert_eq!(directory.folder_cover, Some(PathBuf::from(COVER)));

        // Files never receive a cover, even when their path is a cover entry.
        let mut file = entry(COVER, false);
        attach_folder_cover(&mut file, Some(&covers));
        assert_eq!(file.folder_cover, None);

        let mut unknown_folder = entry(r"Z:\Library\Unknown", true);
        attach_folder_cover(&mut unknown_folder, Some(&covers));
        assert_eq!(unknown_folder.folder_cover, None);

        let mut without_covers = entry(FOLDER, true);
        attach_folder_cover(&mut without_covers, None);
        assert_eq!(without_covers.folder_cover, None);
    }
}
