use crate::domain::file_entry::FileEntry;
use crate::infrastructure::app_state_db::AppStateDb;
use crate::infrastructure::directory_cache::DirectoryCache;
use crate::infrastructure::directory_index::DirectoryIndex;
use crate::infrastructure::disk_cache::ThumbnailDiskCache;
use crate::infrastructure::folder_compose::FolderComposer;
use crate::infrastructure::icon_disk_cache::IconDiskCache;
use crate::infrastructure::onedrive;
use crate::infrastructure::windows as windows_infra;
use crate::workers::thumbnail::{
    new_shared_bulk_thumbnail_progress, spawn_thumbnail_workers, PriorityThumbnailQueue,
    SharedBulkThumbnailProgress,
};
use eframe::egui;
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize};
use std::sync::{mpsc, Arc};
use std::time::Instant;

use super::folder_size_state::FolderSizeMessage;
use super::init_preferences::StartupPreferences;
use super::init_workers::{
    spawn_async_font_loader, spawn_consistency_probe_worker, spawn_cover_worker,
    spawn_disk_cache_invalidation_worker, spawn_file_hash_worker, spawn_file_icon_cache_gc_worker,
    spawn_file_operation_worker, spawn_folder_preview_workers, spawn_folder_size_batch_worker,
    spawn_folder_size_worker, spawn_global_search_worker, spawn_icon_worker,
    spawn_live_file_size_worker, spawn_metadata_worker, spawn_prefetching_workers,
    PrefetchWorkerHandles,
};
use super::state::{FolderLoadError, InactiveItemsRebuildResult, ItemsRebuildResult};

pub(in crate::app) struct AppBootstrap {
    pub(in crate::app) file_entry_sender: mpsc::Sender<(usize, Vec<FileEntry>)>,
    pub(in crate::app) file_entry_receiver: mpsc::Receiver<(usize, Vec<FileEntry>)>,
    pub(in crate::app) folder_load_failure_sender: mpsc::Sender<(usize, FolderLoadError)>,
    pub(in crate::app) folder_load_failure_receiver: mpsc::Receiver<(usize, FolderLoadError)>,
    pub(in crate::app) items_rebuild_sender: mpsc::Sender<ItemsRebuildResult>,
    pub(in crate::app) items_rebuild_receiver: mpsc::Receiver<ItemsRebuildResult>,
    /// EST-02: bounded persistent folder-load worker pool.
    pub(in crate::app) folder_load_pool:
        Arc<crate::app::init_workers::folder_load_pool::FolderLoadPool>,
    pub(in crate::app) inactive_items_rebuild_sender: mpsc::Sender<InactiveItemsRebuildResult>,
    pub(in crate::app) inactive_items_rebuild_receiver: mpsc::Receiver<InactiveItemsRebuildResult>,

    pub(in crate::app) disk_cache: Arc<ThumbnailDiskCache>,
    pub(in crate::app) app_state_db: Arc<AppStateDb>,
    pub(in crate::app) directory_index: Option<Arc<DirectoryIndex>>,
    pub(in crate::app) directory_cache: Arc<DirectoryCache>,
    pub(in crate::app) startup_preferences: StartupPreferences,

    pub(in crate::app) cover_req_tx: mpsc::Sender<PathBuf>,
    pub(in crate::app) cover_res_rx: mpsc::Receiver<(PathBuf, Option<PathBuf>)>,
    #[cfg(feature = "notify-watcher")]
    pub(in crate::app) fs_tx:
        crossbeam_channel::Sender<crate::app::state::TimestampedNotifyEvent>,
    #[cfg(feature = "notify-watcher")]
    pub(in crate::app) fs_rx:
        crossbeam_channel::Receiver<crate::app::state::TimestampedNotifyEvent>,
    pub(in crate::app) device_event_receiver: mpsc::Receiver<()>,

    pub(in crate::app) thumbnail_queue: Arc<PriorityThumbnailQueue>,
    pub(in crate::app) shared_gen: Arc<AtomicUsize>,
    pub(in crate::app) img_rx: crossbeam_channel::Receiver<crate::domain::thumbnail::ThumbnailData>,
    /// EST-05: shutdown flag observed by the thumbnail deferred-retry loop so
    /// the worker fleet can terminate cooperatively (defense in depth on top
    /// of the existing `process::exit` shutdown path).
    pub(in crate::app) thumbnail_pipeline_shutdown: Arc<AtomicBool>,
    pub(in crate::app) pending_deletions: Arc<dashmap::DashMap<PathBuf, ()>>,
    pub(in crate::app) bulk_thumbnail_progress: SharedBulkThumbnailProgress,
    pub(in crate::app) bulk_thumbnail_scanning: Arc<AtomicBool>,
    pub(in crate::app) bulk_thumbnail_total: Arc<AtomicUsize>,
    pub(in crate::app) bulk_thumbnail_completed: Arc<AtomicUsize>,
    pub(in crate::app) bulk_thumbnail_session: Arc<AtomicU64>,
    pub(in crate::app) font_rx: mpsc::Receiver<egui::FontDefinitions>,

    pub(in crate::app) icon_req_tx:
        crossbeam_channel::Sender<crate::app::init_workers::IconRequest>,
    pub(in crate::app) icon_res_rx: mpsc::Receiver<crate::app::init_workers::IconResponse>,
    pub(in crate::app) meta_req_tx: mpsc::Sender<(PathBuf, u64)>,
    pub(in crate::app) meta_res_rx: mpsc::Receiver<(PathBuf, u64, windows_infra::MediaMetadata)>,
    pub(in crate::app) live_size_req_tx: mpsc::Sender<super::live_file_size::LiveFileSizeRequest>,
    pub(in crate::app) live_size_res_rx:
        mpsc::Receiver<super::live_file_size::LiveFileSizeResponse>,
    pub(in crate::app) file_hash_req_tx: mpsc::Sender<super::file_hash::FileHashRequest>,
    pub(in crate::app) file_hash_res_rx: mpsc::Receiver<super::file_hash::FileHashResponse>,
    pub(in crate::app) folder_preview_tx:
        crossbeam_channel::Sender<crate::workers::folder_preview_worker::FolderPreviewRequest>,
    pub(in crate::app) folder_preview_res_rx:
        mpsc::Receiver<crate::workers::folder_preview_worker::FolderPreviewData>,
    pub(in crate::app) folder_preview_trace:
        Arc<crate::workers::folder_preview_worker::FolderPreviewTraceCounters>,
    pub(in crate::app) folder_size_req_tx:
        mpsc::Sender<crate::app::folder_size_state::FolderSizeRequest>,
    pub(in crate::app) folder_size_res_rx: mpsc::Receiver<FolderSizeMessage>,
    pub(in crate::app) folder_size_cancel: Arc<AtomicBool>,
    pub(in crate::app) folder_size_latest_request_id: Arc<AtomicU64>,
    pub(in crate::app) batch_size_tx: mpsc::Sender<crate::app::folder_size_state::BatchSizeRequest>,
    pub(in crate::app) batch_size_rx:
        mpsc::Receiver<crate::app::folder_size_state::BatchSizeResult>,
    pub(in crate::app) batch_size_cancel: Arc<AtomicBool>,
    pub(in crate::app) batch_size_generation: Arc<AtomicU64>,

    pub(in crate::app) prefetch_tx: mpsc::Sender<crate::workers::prefetch_worker::PrefetchMessage>,
    pub(in crate::app) idle_warmup_tx: mpsc::Sender<crate::workers::idle_warmup::IdleWarmupMessage>,

    pub(in crate::app) file_op_tx:
        mpsc::Sender<crate::workers::file_operation_worker::FileOperationRequest>,
    pub(in crate::app) file_op_res_rx:
        mpsc::Receiver<crate::workers::file_operation_worker::FileOperationResult>,
    pub(in crate::app) extraction_progress:
        crate::infrastructure::archive_extract::SharedExtractionProgress,
    pub(in crate::app) extraction_cancel:
        crate::infrastructure::archive_extract::ExtractionCancelFlag,
    pub(in crate::app) global_search_tx:
        mpsc::Sender<crate::workers::global_search_worker::GlobalSearchRequest>,
    pub(in crate::app) global_search_res_rx:
        mpsc::Receiver<crate::workers::global_search_worker::GlobalSearchResponse>,
    pub(in crate::app) disk_cache_invalidation_tx:
        mpsc::Sender<Vec<crate::app::init_workers::CacheInvalidationEntry>>,

    pub(in crate::app) consistency_probe_tx:
        mpsc::Sender<super::init_workers::consistency_probe_worker::ConsistencyProbeRequest>,
    pub(in crate::app) consistency_probe_rx:
        mpsc::Receiver<super::init_workers::consistency_probe_worker::ConsistencyProbeResult>,

    pub(in crate::app) disks: Vec<(String, String)>,
    pub(in crate::app) cloud_roots: Vec<crate::domain::cloud_root::CloudRoot>,
    /// Deferred full drive/cloud detection (runs on background thread to avoid
    /// blocking on sleeping HDDs during cold start). Delivers once, then is dropped.
    pub(in crate::app) cloud_root_rx: mpsc::Receiver<crate::app::drive_state::DriveScanResult>,
    pub(in crate::app) drive_scan_tx: mpsc::Sender<crate::app::drive_state::DriveScanResult>,
    pub(in crate::app) drive_scan_rx: mpsc::Receiver<crate::app::drive_state::DriveScanResult>,
    pub(in crate::app) drive_info_tx: mpsc::Sender<crate::app::drive_state::DriveInfoRefreshResult>,
    pub(in crate::app) drive_info_rx:
        mpsc::Receiver<crate::app::drive_state::DriveInfoRefreshResult>,

    /// Custom composed empty folder icon (back+front+paper_sheet).
    /// Used as the default folder icon instead of the Windows yellow folder.
    pub(in crate::app) custom_folder_icon: (Vec<u8>, u32, u32),
}

/// Extracts a human-readable message from a captured panic payload.
fn describe_panic(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic payload".to_string()
    }
}

/// Resolves a mandatory store that failed its primary constructor.
///
/// Builds an in-memory fallback via `fallback` (a distinct constructor that does
/// not repeat the deterministic failure). Only when even the in-memory fallback
/// cannot be built — i.e. no valid store exists at all — is startup aborted.
fn resolve_mandatory_db<T>(
    component: &str,
    failure: &str,
    fallback: impl FnOnce() -> rusqlite::Result<T>,
) -> T {
    log::error!(
        "[STARTUP] {} primary init failed ({}). Building in-memory fallback.",
        component,
        failure
    );
    match fallback() {
        Ok(store) => {
            log::warn!(
                "[STARTUP] {} running from in-memory fallback (state not persisted this session).",
                component
            );
            store
        }
        Err(e) => {
            log::error!(
                "[STARTUP] {} in-memory fallback also failed: {:?}. No valid store — exiting.",
                component,
                e
            );
            std::process::exit(1);
        }
    }
}

pub(in crate::app) fn bootstrap_app(ctx: &egui::Context) -> AppBootstrap {
    let bootstrap_start = Instant::now();
    let mut last_step = bootstrap_start;
    macro_rules! log_bootstrap_step {
        ($label:literal) => {{
            let now = Instant::now();
            log::info!(
                "[STARTUP] bootstrap {} step_ms={} total_ms={}",
                $label,
                now.duration_since(last_step).as_millis(),
                now.duration_since(bootstrap_start).as_millis()
            );
            last_step = now;
        }};
    }

    // Worker results carry decoded RGBA buffers. At the largest thumbnail
    // bucket (512px), a single result can be ~1 MiB, so a large channel turns
    // into hidden working-set growth outside the visible pending/RGBA caches.
    // Keep this small enough to provide backpressure to decoder workers while
    // still allowing the UI upload loop to batch a few frames worth of results.
    const THUMBNAIL_RESULT_CHANNEL_CAPACITY: usize = 32;

    if let Err(error) = crate::infrastructure::virtual_drive_config::ensure_config_exists() {
        log::warn!(
            "[Config] Failed to initialize virtual drive configuration: {}",
            error
        );
    }
    log_bootstrap_step!("virtual_drive_config");

    let (file_entry_sender, file_entry_receiver) = mpsc::channel::<(usize, Vec<FileEntry>)>();
    let (folder_load_failure_sender, folder_load_failure_receiver) =
        mpsc::channel::<(usize, FolderLoadError)>();
    let (items_rebuild_sender, items_rebuild_receiver) = mpsc::channel::<ItemsRebuildResult>();
    // EST-02: fixed-size pool for folder-load pipelines.
    let folder_load_pool = Arc::new(crate::app::init_workers::folder_load_pool::FolderLoadPool::new());
    let (inactive_items_rebuild_sender, inactive_items_rebuild_receiver) =
        mpsc::channel::<InactiveItemsRebuildResult>();

    let cache_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("MTT-File-Manager")
        .join("thumbnails");
    let base_dir = cache_dir.parent().unwrap_or(&cache_dir).to_path_buf();
    let state_dir = base_dir.join("state");
    let dir_cache_dir = base_dir.join("cache");
    let _ = std::fs::create_dir_all(&dir_cache_dir);

    // PERF: Parallelize independent SQLite opens + IconDiskCache on cold start.
    // Each opens its own DB file with no cross-dependency, so running them
    // concurrently turns the SUM of their latencies into the MAX.
    //
    // BUG-A1: every join is handled individually so a recoverable panic in one
    // bootstrap thread degrades that single component instead of unwinding into
    // the main thread and aborting the whole application.
    let parallel_start = Instant::now();
    let (disk_cache, app_state_db, directory_index, icon_disk_cache): (
        Arc<ThumbnailDiskCache>,
        Arc<AppStateDb>,
        Option<Arc<DirectoryIndex>>,
        Arc<IconDiskCache>,
    ) = std::thread::scope(|s| {
        let disk_cache_handle = s.spawn(|| ThumbnailDiskCache::new(cache_dir.clone()));
        let app_state_handle = s.spawn(|| AppStateDb::new(state_dir.clone()));
        let dir_index_handle =
            s.spawn(|| DirectoryIndex::open(&dir_cache_dir.join("directory_cache.db")));
        let icon_cache_handle = s.spawn(|| IconDiskCache::new(&base_dir));

        // AppStateDb: mandatory. On open error or panic, build an in-memory
        // store via a distinct constructor instead of retrying the primary one
        // (a deterministic panic would just repeat).
        let app_state_db = match app_state_handle.join() {
            Ok(Ok(db)) => db,
            Ok(Err(e)) => resolve_mandatory_db(
                "AppStateDb",
                &format!("open error: {:?}", e),
                AppStateDb::new_in_memory,
            ),
            Err(panic) => resolve_mandatory_db(
                "AppStateDb",
                &format!("panic: {}", describe_panic(panic.as_ref())),
                AppStateDb::new_in_memory,
            ),
        };

        // ThumbnailDiskCache: mandatory, same degradation policy.
        let disk_cache = match disk_cache_handle.join() {
            Ok(Ok(c)) => c,
            Ok(Err(e)) => resolve_mandatory_db(
                "ThumbnailDiskCache",
                &format!("open error: {:?}", e),
                ThumbnailDiskCache::new_in_memory,
            ),
            Err(panic) => resolve_mandatory_db(
                "ThumbnailDiskCache",
                &format!("panic: {}", describe_panic(panic.as_ref())),
                ThumbnailDiskCache::new_in_memory,
            ),
        };

        // DirectoryIndex: optional. Degrade to None on open error or panic,
        // matching the existing behavior for normal open failures.
        let directory_index = match dir_index_handle.join() {
            Ok(Ok(index)) => Some(Arc::new(index)),
            Ok(Err(e)) => {
                log::warn!("[STARTUP] DirectoryIndex disabled — open error: {:?}", e);
                None
            }
            Err(panic) => {
                log::error!(
                    "[STARTUP] DirectoryIndex disabled — thread panicked: {}",
                    describe_panic(panic.as_ref())
                );
                None
            }
        };

        // IconDiskCache: degradable to a session-only in-memory store.
        let icon_disk_cache = match icon_cache_handle.join() {
            Ok(cache) => cache,
            Err(panic) => {
                log::error!(
                    "[STARTUP] IconDiskCache degraded to in-memory — thread panicked: {}",
                    describe_panic(panic.as_ref())
                );
                IconDiskCache::in_memory_fallback()
            }
        };

        (
            Arc::new(disk_cache),
            Arc::new(app_state_db),
            directory_index,
            Arc::new(icon_disk_cache),
        )
    });

    // Legacy migration runs only after the active stores are finalized, and only
    // when BOTH mandatory stores are on their primary path. Migrating while a
    // store is in fallback would rewrite a primary DB the app is not using.
    if app_state_db.is_on_primary_path() && disk_cache.is_on_primary_path() {
        if let Err(error) = migrate_legacy_tables(
            &cache_dir.join("thumbnails.db"),
            &state_dir.join("app_state.db"),
        ) {
            log::error!(
                "[Migration] Failed to migrate legacy tables; source tables were preserved: {:?}",
                error
            );
        }
    } else {
        log::warn!(
            "[Migration] Skipped — a mandatory store is in fallback mode \
             (app_state primary={}, thumbnails primary={})",
            app_state_db.is_on_primary_path(),
            disk_cache.is_on_primary_path()
        );
    }

    log::info!(
        "[STARTUP] parallel SQLite opens + migration total_ms={}",
        parallel_start.elapsed().as_millis()
    );

    let (cover_req_tx, cover_res_rx) = spawn_cover_worker(app_state_db.clone());
    // EST-01: bounded intake for filesystem events. On saturation the producer
    // drops the oldest queued event and coalesces the burst into a single
    // overflow marker, so a sustained event storm cannot grow this queue
    // monotonically (memory) or keep the UI awake indefinitely.
    #[cfg(feature = "notify-watcher")]
    const FS_EVENT_CHANNEL_CAPACITY: usize = 4096;
    #[cfg(feature = "notify-watcher")]
    let (fs_tx, fs_rx) = crossbeam_channel::bounded(FS_EVENT_CHANNEL_CAPACITY);
    let (device_event_sender, device_event_receiver) = mpsc::channel();
    windows_infra::start_device_change_listener(device_event_sender, ctx.clone());
    log_bootstrap_step!("base_channels_and_device_listener");

    // PERF: Defer FolderComposer PNG decoding to a background thread.
    // Decodes 3 embedded PNGs + composites the empty-folder icon (~30-80ms cold).
    // Runs concurrently with worker spawning below.
    let folder_composer_handle = std::thread::spawn(|| {
        FolderComposer::try_new().map(|composer| {
            let empty_icon = composer.compose_empty();
            (composer, empty_icon)
        })
    });

    let (img_tx, img_rx) = crossbeam_channel::bounded(THUMBNAIL_RESULT_CHANNEL_CAPACITY);
    let thumbnail_queue = Arc::new(PriorityThumbnailQueue::new());
    let thumbnail_pipeline_shutdown = Arc::new(AtomicBool::new(false));
    let shared_gen = Arc::new(AtomicUsize::new(0));
    let bulk_thumbnail_progress = new_shared_bulk_thumbnail_progress();
    let bulk_thumbnail_scanning = Arc::new(AtomicBool::new(false));
    let bulk_thumbnail_total = Arc::new(AtomicUsize::new(0));
    let bulk_thumbnail_completed = Arc::new(AtomicUsize::new(0));
    let bulk_thumbnail_session = Arc::new(AtomicU64::new(0));

    onedrive::init_onedrive_paths();
    let directory_cache = Arc::new(DirectoryCache::new());
    let startup_preferences = StartupPreferences::load(&app_state_db);
    let font_rx = spawn_async_font_loader();
    log_bootstrap_step!("onedrive_preferences_fonts");

    let pending_deletions: Arc<dashmap::DashMap<PathBuf, ()>> = Arc::new(dashmap::DashMap::new());
    spawn_thumbnail_workers(
        thumbnail_queue.clone(),
        img_tx,
        ctx.clone(),
        shared_gen.clone(),
        disk_cache.clone(),
        pending_deletions.clone(),
        bulk_thumbnail_progress.clone(),
        bulk_thumbnail_completed.clone(),
        bulk_thumbnail_session.clone(),
        thumbnail_pipeline_shutdown.clone(),
    );
    log_bootstrap_step!("thumbnail_workers");

    spawn_file_icon_cache_gc_worker(icon_disk_cache.clone());
    let (icon_req_tx, icon_res_rx) = spawn_icon_worker(ctx, shared_gen.clone(), icon_disk_cache);

    let (meta_req_tx, meta_res_rx) = spawn_metadata_worker(ctx);
    let (live_size_req_tx, live_size_res_rx) = spawn_live_file_size_worker(ctx);
    let (file_hash_req_tx, file_hash_res_rx) = spawn_file_hash_worker(ctx);
    // Folder preview is a degradable component: if the composer fails to decode
    // its embedded layers (or its thread panics), disable preview and fall back
    // to a valid placeholder folder icon instead of aborting startup.
    let (folder_composer, custom_folder_icon) = match folder_composer_handle.join() {
        Ok(Some((composer, icon))) => (Some(Arc::new(composer)), icon),
        Ok(None) => {
            log::error!(
                "[STARTUP] FolderComposer init failed — folder preview disabled, using placeholder icon"
            );
            (
                None,
                crate::infrastructure::folder_compose::placeholder_folder_icon(),
            )
        }
        Err(panic) => {
            log::error!(
                "[STARTUP] FolderComposer thread panicked: {} — folder preview disabled",
                describe_panic(panic.as_ref())
            );
            (
                None,
                crate::infrastructure::folder_compose::placeholder_folder_icon(),
            )
        }
    };
    let folder_preview_trace =
        Arc::new(crate::workers::folder_preview_worker::FolderPreviewTraceCounters::default());
    let (folder_preview_tx, folder_preview_res_rx) = spawn_folder_preview_workers(
        ctx,
        disk_cache.clone(),
        folder_composer,
        folder_preview_trace.clone(),
    );
    let (folder_size_req_tx, folder_size_res_rx, folder_size_cancel, folder_size_latest_request_id) =
        spawn_folder_size_worker(ctx);
    let (batch_size_tx, batch_size_rx, batch_size_cancel, batch_size_generation) =
        spawn_folder_size_batch_worker(ctx);
    log_bootstrap_step!("visual_and_size_workers");

    let PrefetchWorkerHandles {
        prefetch_sender: prefetch_tx,
        idle_warmup_sender: idle_warmup_tx,
    } = spawn_prefetching_workers(
        directory_cache.clone(),
        thumbnail_queue.clone(),
        shared_gen.clone(),
    );

    let (file_op_tx, file_op_res_rx, extraction_progress, extraction_cancel) =
        spawn_file_operation_worker();
    let (global_search_tx, global_search_res_rx) = spawn_global_search_worker(ctx);
    let disk_cache_invalidation_tx =
        spawn_disk_cache_invalidation_worker(disk_cache.clone(), app_state_db.clone());
    let (consistency_probe_tx, consistency_probe_rx) = spawn_consistency_probe_worker(ctx.clone());
    log_bootstrap_step!("pipeline_and_search_workers");

    // PERF: Start with drive roots only. Full labels/cloud roots can touch
    // sleeping volumes, so they are refreshed on a background thread below.
    let disks = windows_infra::get_all_drives_fast();
    let (cloud_root_tx, cloud_root_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let (disks, cloud_roots, unavailable_label_roots) =
            windows_infra::get_drives_and_cloud_roots();
        let _ = cloud_root_tx.send(crate::app::drive_state::DriveScanResult {
            disks,
            cloud_roots,
            unavailable_label_roots,
        });
    });
    let (drive_scan_tx, drive_scan_rx) = mpsc::channel();
    let (drive_info_tx, drive_info_rx) = mpsc::channel();
    log_bootstrap_step!("drives_and_cloud_roots");
    let _ = last_step;

    AppBootstrap {
        file_entry_sender,
        file_entry_receiver,
        folder_load_failure_sender,
        folder_load_failure_receiver,
        items_rebuild_sender,
        items_rebuild_receiver,
        folder_load_pool,
        inactive_items_rebuild_sender,
        inactive_items_rebuild_receiver,
        disk_cache,
        app_state_db,
        directory_index,
        directory_cache,
        startup_preferences,
        cover_req_tx,
        cover_res_rx,
        #[cfg(feature = "notify-watcher")]
        fs_tx,
        #[cfg(feature = "notify-watcher")]
        fs_rx,
        device_event_receiver,
        thumbnail_queue,
        thumbnail_pipeline_shutdown,
        shared_gen,
        img_rx,
        pending_deletions,
        bulk_thumbnail_progress,
        bulk_thumbnail_scanning,
        bulk_thumbnail_total,
        bulk_thumbnail_completed,
        bulk_thumbnail_session,
        font_rx,
        icon_req_tx,
        icon_res_rx,
        meta_req_tx,
        meta_res_rx,
        live_size_req_tx,
        live_size_res_rx,
        file_hash_req_tx,
        file_hash_res_rx,
        folder_preview_tx,
        folder_preview_res_rx,
        folder_preview_trace,
        folder_size_req_tx,
        folder_size_res_rx,
        folder_size_cancel,
        folder_size_latest_request_id,
        batch_size_tx,
        batch_size_rx,
        batch_size_cancel,
        batch_size_generation,
        prefetch_tx,
        idle_warmup_tx,
        file_op_tx,
        file_op_res_rx,
        extraction_progress,
        extraction_cancel,
        global_search_tx,
        global_search_res_rx,
        disk_cache_invalidation_tx,
        consistency_probe_tx,
        consistency_probe_rx,
        disks,
        cloud_roots: Vec::new(),
        cloud_root_rx,
        drive_scan_tx,
        drive_scan_rx,
        drive_info_tx,
        drive_info_rx,
        custom_folder_icon,
    }
}

/// One-time migration: copy user_preferences, folder_locks, pinned_folders,
/// folder_covers from old `thumbnails.db` into the new `app_state.db`, then
/// drop the migrated tables (plus orphaned directory_index / file_index).
///
/// Uses `ATTACH DATABASE` so all copying happens in a single SQLite session.
/// `INSERT OR IGNORE` ensures no data is overwritten if the new DB already
/// has rows (e.g. from a previous successful migration).
fn migrate_legacy_tables(
    thumbnails_db_path: &Path,
    app_state_db_path: &Path,
) -> Result<usize, String> {
    let mut conn = Connection::open(thumbnails_db_path)
        .map_err(|error| format!("open legacy database: {error}"))?;

    // Check whether any legacy table still exists in thumbnails.db.
    let has_legacy: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN \
                 ('user_preferences','folder_locks','pinned_folders','folder_covers')",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("inspect legacy schema: {error}"))?
        > 0;

    if !has_legacy {
        return Ok(0); // Already migrated or fresh install — nothing to do.
    }

    log::info!(
        "[Migration] Legacy tables detected in {:?} — migrating to {:?}",
        thumbnails_db_path,
        app_state_db_path
    );

    let attach_path = app_state_db_path.to_string_lossy();
    conn.execute("ATTACH DATABASE ?1 AS new_state", [attach_path.as_ref()])
        .map_err(|error| format!("attach destination database: {error}"))?;

    let mut total_migrated: usize = 0;
    let copies = [
        (
            "user_preferences",
            "INSERT OR IGNORE INTO new_state.user_preferences (key, value) \
             SELECT key, value FROM user_preferences",
            "SELECT COUNT(*) FROM user_preferences source WHERE NOT EXISTS (\
             SELECT 1 FROM new_state.user_preferences destination \
             WHERE destination.key = source.key)",
        ),
        (
            "folder_covers",
            "INSERT OR IGNORE INTO new_state.folder_covers (folder_path, cover_path) \
             SELECT folder_path, cover_path FROM folder_covers",
            "SELECT COUNT(*) FROM folder_covers source WHERE NOT EXISTS (\
             SELECT 1 FROM new_state.folder_covers destination \
             WHERE destination.folder_path = source.folder_path)",
        ),
        (
            "folder_locks",
            "INSERT OR IGNORE INTO new_state.folder_locks \
             (path, view_mode, sort_mode, sort_descending, folders_position) \
             SELECT path, view_mode, sort_mode, sort_descending, folders_position \
             FROM folder_locks",
            "SELECT COUNT(*) FROM folder_locks source WHERE NOT EXISTS (\
             SELECT 1 FROM new_state.folder_locks destination \
             WHERE destination.path = source.path)",
        ),
        (
            "pinned_folders",
            "INSERT OR IGNORE INTO new_state.pinned_folders (path, display_name, position) \
             SELECT path, display_name, position FROM pinned_folders",
            "SELECT COUNT(*) FROM pinned_folders source WHERE NOT EXISTS (\
             SELECT 1 FROM new_state.pinned_folders destination \
             WHERE destination.path = source.path)",
        ),
    ];

    // Phase 1 writes only the destination. If the process stops after this
    // commit, the source remains intact and the migration can safely rerun.
    {
        let tx = conn
            .transaction()
            .map_err(|error| format!("begin destination migration: {error}"))?;
        for (table, copy_sql, _) in copies {
            let exists = tx
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [table],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|error| format!("inspect legacy table {table}: {error}"))?
                > 0;
            if !exists {
                continue;
            }

            let copied = tx.execute(copy_sql, []).map_err(|error| {
                log::error!(
                    "[Migration] Failed to copy legacy table {}; rolling back destination: {:?}",
                    table,
                    error
                );
                format!("copy legacy table {table}: {error}")
            })?;
            log::info!("[Migration] {}: {} rows staged", table, copied);
            total_migrated += copied;
        }
        tx.commit()
            .map_err(|error| format!("commit destination migration: {error}"))?;
    }

    // Verify durable destination coverage before touching the source. Matching
    // primary keys are sufficient because INSERT OR IGNORE intentionally keeps
    // values already written by a prior successful migration.
    for (table, _, verify_sql) in copies {
        let exists = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                [table],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| format!("inspect legacy table {table}: {error}"))?
            > 0;
        if !exists {
            continue;
        }
        let missing: i64 = conn
            .query_row(verify_sql, [], |row| row.get(0))
            .map_err(|error| format!("verify migrated table {table}: {error}"))?;
        if missing != 0 {
            return Err(format!(
                "destination verification failed for {table}: {missing} source rows missing"
            ));
        }
    }

    conn.execute_batch("DETACH DATABASE new_state")
        .map_err(|error| format!("detach destination database: {error}"))?;

    // Phase 2 mutates only the source database, so its commit is crash-atomic
    // even when both production databases use WAL.
    let cleanup = conn
        .transaction()
        .map_err(|error| format!("begin legacy cleanup: {error}"))?;
    cleanup
        .execute_batch(
            "DROP TABLE IF EXISTS user_preferences;
             DROP TABLE IF EXISTS folder_locks;
             DROP TABLE IF EXISTS pinned_folders;
             DROP TABLE IF EXISTS folder_covers;
             DROP TABLE IF EXISTS directory_index;
             DROP TABLE IF EXISTS file_index;",
        )
        .map_err(|error| format!("drop migrated legacy tables: {error}"))?;
    cleanup
        .commit()
        .map_err(|error| format!("commit legacy cleanup: {error}"))?;

    log::info!(
        "[Migration] Complete — {} total rows migrated from thumbnails.db → app_state.db",
        total_migrated
    );
    Ok(total_migrated)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_destination(path: &Path) -> Connection {
        let conn = Connection::open(path).expect("open destination database");
        conn.execute_batch(
            "CREATE TABLE user_preferences (key TEXT PRIMARY KEY, value TEXT);
             CREATE TABLE folder_covers (folder_path TEXT PRIMARY KEY, cover_path TEXT);
             CREATE TABLE folder_locks (
                 path TEXT PRIMARY KEY,
                 view_mode TEXT NOT NULL,
                 sort_mode TEXT NOT NULL,
                 sort_descending TEXT NOT NULL,
                 folders_position TEXT NOT NULL
             );
             CREATE TABLE pinned_folders (
                 path TEXT PRIMARY KEY,
                 display_name TEXT NOT NULL,
                 position INTEGER NOT NULL DEFAULT 0
             );",
        )
        .expect("create destination schema");
        conn
    }

    fn table_exists(conn: &Connection, table: &str) -> bool {
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
            [table],
            |row| row.get(0),
        )
        .expect("query table existence")
    }

    #[test]
    fn migrate_legacy_tables_copies_rows_and_drops_sources() {
        let temp = tempfile::tempdir().expect("create temporary directory");
        let source_path = temp.path().join("thumbnails.db");
        let destination_path = temp.path().join("app_state.db");
        let source = Connection::open(&source_path).expect("open source database");
        source
            .execute_batch(
                "CREATE TABLE user_preferences (key TEXT PRIMARY KEY, value TEXT);
                 INSERT INTO user_preferences VALUES ('theme', 'dark');
                 CREATE TABLE folder_covers (folder_path TEXT PRIMARY KEY, cover_path TEXT);
                 INSERT INTO folder_covers VALUES ('C:\\Pictures', 'cover.jpg');
                 CREATE TABLE folder_locks (
                     path TEXT PRIMARY KEY,
                     view_mode TEXT NOT NULL,
                     sort_mode TEXT NOT NULL,
                     sort_descending TEXT NOT NULL,
                     folders_position TEXT NOT NULL,
                     search_query TEXT
                 );
                 INSERT INTO folder_locks VALUES ('C:\\Work', 'grid', 'name', 'false', 'first', NULL);
                 CREATE TABLE pinned_folders (
                     path TEXT PRIMARY KEY,
                     display_name TEXT NOT NULL,
                     position INTEGER NOT NULL
                 );
                 INSERT INTO pinned_folders VALUES ('C:\\Work', 'Work', 1);
                 CREATE TABLE directory_index (path TEXT);
                 CREATE TABLE file_index (path TEXT);",
            )
            .expect("create source schema and rows");
        drop(source);
        let destination = create_destination(&destination_path);
        drop(destination);

        assert_eq!(
            migrate_legacy_tables(&source_path, &destination_path).expect("migrate legacy tables"),
            4
        );

        let source = Connection::open(&source_path).expect("reopen source database");
        for table in [
            "user_preferences",
            "folder_covers",
            "folder_locks",
            "pinned_folders",
            "directory_index",
            "file_index",
        ] {
            assert!(
                !table_exists(&source, table),
                "source table {table} remains"
            );
        }
        let destination = Connection::open(&destination_path).expect("reopen destination database");
        for table in [
            "user_preferences",
            "folder_covers",
            "folder_locks",
            "pinned_folders",
        ] {
            let count: i64 = destination
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .expect("count migrated rows");
            assert_eq!(count, 1, "destination table {table}");
        }
    }

    #[test]
    fn migrate_legacy_tables_is_idempotent_and_preserves_destination_rows() {
        let temp = tempfile::tempdir().expect("create temporary directory");
        let source_path = temp.path().join("thumbnails.db");
        let destination_path = temp.path().join("app_state.db");
        let source = Connection::open(&source_path).expect("open source database");
        source
            .execute_batch(
                "CREATE TABLE user_preferences (key TEXT PRIMARY KEY, value TEXT);
                 INSERT INTO user_preferences VALUES ('theme', 'source');",
            )
            .expect("create source row");
        drop(source);
        let destination = create_destination(&destination_path);
        destination
            .execute(
                "INSERT INTO user_preferences VALUES ('theme', 'destination')",
                [],
            )
            .expect("create existing destination row");
        drop(destination);

        assert_eq!(
            migrate_legacy_tables(&source_path, &destination_path).expect("first migration"),
            0
        );
        assert_eq!(
            migrate_legacy_tables(&source_path, &destination_path).expect("second migration"),
            0
        );

        let destination = Connection::open(&destination_path).expect("reopen destination database");
        let value: String = destination
            .query_row(
                "SELECT value FROM user_preferences WHERE key='theme'",
                [],
                |row| row.get(0),
            )
            .expect("read destination row");
        assert_eq!(value, "destination");
    }

    #[test]
    fn migrate_legacy_tables_rolls_back_and_preserves_sources_on_copy_failure() {
        let temp = tempfile::tempdir().expect("create temporary directory");
        let source_path = temp.path().join("thumbnails.db");
        let destination_path = temp.path().join("app_state.db");
        let source = Connection::open(&source_path).expect("open source database");
        source
            .execute_batch(
                "CREATE TABLE user_preferences (key TEXT PRIMARY KEY, value TEXT);
                 INSERT INTO user_preferences VALUES ('theme', 'dark');
                 CREATE TABLE folder_covers (folder_path TEXT PRIMARY KEY);
                 INSERT INTO folder_covers VALUES ('C:\\Pictures');",
            )
            .expect("create malformed legacy schema");
        drop(source);
        let destination = create_destination(&destination_path);
        drop(destination);

        assert!(migrate_legacy_tables(&source_path, &destination_path).is_err());

        let source = Connection::open(&source_path).expect("reopen source database");
        assert!(table_exists(&source, "user_preferences"));
        assert!(table_exists(&source, "folder_covers"));
        let source_preferences: i64 = source
            .query_row("SELECT COUNT(*) FROM user_preferences", [], |row| {
                row.get(0)
            })
            .expect("count preserved source rows");
        assert_eq!(source_preferences, 1);
        let source_covers: i64 = source
            .query_row("SELECT COUNT(*) FROM folder_covers", [], |row| row.get(0))
            .expect("count preserved malformed source rows");
        assert_eq!(source_covers, 1);

        let destination = Connection::open(&destination_path).expect("reopen destination database");
        let destination_preferences: i64 = destination
            .query_row("SELECT COUNT(*) FROM user_preferences", [], |row| {
                row.get(0)
            })
            .expect("count rolled-back destination rows");
        assert_eq!(destination_preferences, 0);
    }
}
