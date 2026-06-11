use crate::domain::file_entry::DriveInfo;
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

use super::folder_size_state::FolderSizeMessage;
use super::init_preferences::StartupPreferences;
use super::init_workers::{
    spawn_async_font_loader, spawn_consistency_probe_worker, spawn_cover_worker,
    spawn_disk_cache_invalidation_worker, spawn_file_icon_cache_gc_worker,
    spawn_file_operation_worker, spawn_folder_preview_workers, spawn_folder_size_batch_worker,
    spawn_folder_size_worker, spawn_global_search_worker, spawn_icon_worker,
    spawn_live_file_size_worker, spawn_metadata_worker, spawn_prefetching_workers,
    PrefetchWorkerHandles,
};
use super::state::{FolderLoadError, ItemsRebuildResult};

pub(in crate::app) struct AppBootstrap {
    pub(in crate::app) file_entry_sender: mpsc::Sender<(usize, Vec<FileEntry>)>,
    pub(in crate::app) file_entry_receiver: mpsc::Receiver<(usize, Vec<FileEntry>)>,
    pub(in crate::app) folder_load_failure_sender: mpsc::Sender<(usize, FolderLoadError)>,
    pub(in crate::app) folder_load_failure_receiver: mpsc::Receiver<(usize, FolderLoadError)>,
    pub(in crate::app) items_rebuild_sender: mpsc::Sender<ItemsRebuildResult>,
    pub(in crate::app) items_rebuild_receiver: mpsc::Receiver<ItemsRebuildResult>,

    pub(in crate::app) disk_cache: Arc<ThumbnailDiskCache>,
    pub(in crate::app) app_state_db: Arc<AppStateDb>,
    pub(in crate::app) directory_index: Option<Arc<DirectoryIndex>>,
    pub(in crate::app) directory_cache: Arc<DirectoryCache>,
    pub(in crate::app) startup_preferences: StartupPreferences,

    pub(in crate::app) cover_req_tx: mpsc::Sender<PathBuf>,
    pub(in crate::app) cover_res_rx: mpsc::Receiver<(PathBuf, Option<PathBuf>)>,
    #[cfg(feature = "notify-watcher")]
    pub(in crate::app) fs_tx: mpsc::Sender<crate::app::state::TimestampedNotifyEvent>,
    #[cfg(feature = "notify-watcher")]
    pub(in crate::app) fs_rx: mpsc::Receiver<crate::app::state::TimestampedNotifyEvent>,
    pub(in crate::app) device_event_receiver: mpsc::Receiver<()>,

    pub(in crate::app) thumbnail_queue: Arc<PriorityThumbnailQueue>,
    pub(in crate::app) shared_gen: Arc<AtomicUsize>,
    pub(in crate::app) img_rx: crossbeam_channel::Receiver<crate::domain::thumbnail::ThumbnailData>,
    pub(in crate::app) pending_deletions: Arc<dashmap::DashMap<PathBuf, ()>>,
    pub(in crate::app) bulk_thumbnail_progress: SharedBulkThumbnailProgress,
    pub(in crate::app) bulk_thumbnail_scanning: Arc<AtomicBool>,
    pub(in crate::app) bulk_thumbnail_total: Arc<AtomicUsize>,
    pub(in crate::app) bulk_thumbnail_completed: Arc<AtomicUsize>,
    pub(in crate::app) bulk_thumbnail_session: Arc<AtomicU64>,
    pub(in crate::app) font_rx: mpsc::Receiver<egui::FontDefinitions>,

    pub(in crate::app) icon_req_tx: mpsc::Sender<(PathBuf, usize)>,
    pub(in crate::app) icon_res_rx: mpsc::Receiver<(PathBuf, usize, Vec<u8>, u32, u32)>,
    pub(in crate::app) meta_req_tx: mpsc::Sender<(PathBuf, u64)>,
    pub(in crate::app) meta_res_rx: mpsc::Receiver<(PathBuf, u64, windows_infra::MediaMetadata)>,
    pub(in crate::app) live_size_req_tx: mpsc::Sender<super::live_file_size::LiveFileSizeRequest>,
    pub(in crate::app) live_size_res_rx:
        mpsc::Receiver<super::live_file_size::LiveFileSizeResponse>,
    pub(in crate::app) folder_preview_tx:
        crossbeam_channel::Sender<crate::workers::folder_preview_worker::FolderPreviewRequest>,
    pub(in crate::app) folder_preview_res_rx:
        mpsc::Receiver<crate::workers::folder_preview_worker::FolderPreviewData>,
    pub(in crate::app) folder_preview_trace:
        Arc<crate::workers::folder_preview_worker::FolderPreviewTraceCounters>,
    pub(in crate::app) folder_size_req_tx: mpsc::Sender<PathBuf>,
    pub(in crate::app) folder_size_res_rx: mpsc::Receiver<FolderSizeMessage>,
    pub(in crate::app) folder_size_cancel: Arc<AtomicBool>,
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
    pub(in crate::app) drive_scan_tx: mpsc::Sender<Vec<(String, String)>>,
    pub(in crate::app) drive_scan_rx: mpsc::Receiver<Vec<(String, String)>>,
    pub(in crate::app) drive_info_tx: mpsc::Sender<Vec<(String, DriveInfo)>>,
    pub(in crate::app) drive_info_rx: mpsc::Receiver<Vec<(String, DriveInfo)>>,

    /// Custom composed empty folder icon (back+front+paper_sheet).
    /// Used as the default folder icon instead of the Windows yellow folder.
    pub(in crate::app) custom_folder_icon: (Vec<u8>, u32, u32),
}

pub(in crate::app) fn bootstrap_app(ctx: &egui::Context) -> AppBootstrap {
    // Worker results carry decoded RGBA buffers. At the largest thumbnail
    // bucket (1024px), a single result can be ~4 MiB, so a large channel turns
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

    let (file_entry_sender, file_entry_receiver) = mpsc::channel::<(usize, Vec<FileEntry>)>();
    let (folder_load_failure_sender, folder_load_failure_receiver) =
        mpsc::channel::<(usize, FolderLoadError)>();
    let (items_rebuild_sender, items_rebuild_receiver) = mpsc::channel::<ItemsRebuildResult>();

    let cache_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("MTT-File-Manager")
        .join("thumbnails");
    let disk_cache = Arc::new(match ThumbnailDiskCache::new(cache_dir.clone()) {
        Ok(cache) => cache,
        Err(e) => {
            log::error!(
                "[Cache] Failed to initialize thumbnail cache at {:?}: {:?}. Retrying in-memory.",
                cache_dir,
                e
            );
            // Last-resort in-memory fallback — thumbnails won't persist but app keeps running.
            ThumbnailDiskCache::new(std::env::temp_dir().join("mtt-cache-fallback")).unwrap_or_else(
                |e2| {
                    log::error!("[Cache] In-memory fallback also failed: {:?}. Exiting.", e2);
                    std::process::exit(1);
                },
            )
        }
    });
    let base_dir = cache_dir.parent().unwrap_or(&cache_dir).to_path_buf();

    let state_dir = base_dir.join("state");
    let app_state_db = Arc::new(match AppStateDb::new(state_dir.clone()) {
        Ok(db) => db,
        Err(e) => {
            log::error!(
                "[State] Failed to initialize app state DB at {:?}: {:?}. Retrying in-memory.",
                state_dir,
                e
            );
            AppStateDb::new(std::env::temp_dir().join("mtt-state-fallback")).unwrap_or_else(|e2| {
                log::error!("[State] In-memory fallback also failed: {:?}. Exiting.", e2);
                std::process::exit(1);
            })
        }
    });

    // One-time migration: move legacy tables from thumbnails.db → app_state.db
    migrate_legacy_tables(
        &cache_dir.join("thumbnails.db"),
        &state_dir.join("app_state.db"),
    );

    let dir_cache_dir = base_dir.join("cache");
    let _ = std::fs::create_dir_all(&dir_cache_dir);
    let directory_index = match DirectoryIndex::open(&dir_cache_dir.join("directory_cache.db")) {
        Ok(index) => Some(Arc::new(index)),
        Err(e) => {
            log::warn!("[Cache] Failed to open directory index: {:?}", e);
            None
        }
    };

    let (cover_req_tx, cover_res_rx) = spawn_cover_worker(app_state_db.clone());
    #[cfg(feature = "notify-watcher")]
    let (fs_tx, fs_rx) = mpsc::channel();
    let (device_event_sender, device_event_receiver) = mpsc::channel();
    windows_infra::start_device_change_listener(device_event_sender, ctx.clone());

    let (img_tx, img_rx) = crossbeam_channel::bounded(THUMBNAIL_RESULT_CHANNEL_CAPACITY);
    let thumbnail_queue = Arc::new(PriorityThumbnailQueue::new());
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
    );

    let icon_disk_cache = Arc::new(IconDiskCache::new(&base_dir));
    spawn_file_icon_cache_gc_worker(icon_disk_cache.clone());
    let (icon_req_tx, icon_res_rx) = spawn_icon_worker(ctx, shared_gen.clone(), icon_disk_cache);

    let (meta_req_tx, meta_res_rx) = spawn_metadata_worker(ctx);
    let (live_size_req_tx, live_size_res_rx) = spawn_live_file_size_worker(ctx);
    let folder_composer = Arc::new(FolderComposer::new());
    let folder_preview_trace =
        Arc::new(crate::workers::folder_preview_worker::FolderPreviewTraceCounters::default());
    // Compose the custom empty folder icon ONCE before sharing the composer.
    let custom_folder_icon = folder_composer.compose_empty();
    let (folder_preview_tx, folder_preview_res_rx) = spawn_folder_preview_workers(
        ctx,
        disk_cache.clone(),
        folder_composer,
        folder_preview_trace.clone(),
    );
    let (folder_size_req_tx, folder_size_res_rx, folder_size_cancel) =
        spawn_folder_size_worker(ctx);
    let (batch_size_tx, batch_size_rx, batch_size_cancel, batch_size_generation) =
        spawn_folder_size_batch_worker(ctx);

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

    let disks = windows_infra::get_all_drives();
    let (drive_scan_tx, drive_scan_rx) = mpsc::channel();
    let (drive_info_tx, drive_info_rx) = mpsc::channel();

    AppBootstrap {
        file_entry_sender,
        file_entry_receiver,
        folder_load_failure_sender,
        folder_load_failure_receiver,
        items_rebuild_sender,
        items_rebuild_receiver,
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
        folder_preview_tx,
        folder_preview_res_rx,
        folder_preview_trace,
        folder_size_req_tx,
        folder_size_res_rx,
        folder_size_cancel,
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
        drive_scan_tx,
        drive_scan_rx,
        drive_info_tx,
        drive_info_rx,
        custom_folder_icon,
    }
}

/// One-time migration: copy user_preferences, folder_locks, pinned_folders,
/// folder_covers from old `thumbnails.db` into the new `app_state.db`, then
/// drop the migrated tables (plus orphaned directory_index / file_index)
/// and VACUUM the old database.
///
/// Uses `ATTACH DATABASE` so all copying happens in a single SQLite session.
/// `INSERT OR IGNORE` ensures no data is overwritten if the new DB already
/// has rows (e.g. from a previous successful migration).
fn migrate_legacy_tables(thumbnails_db_path: &Path, app_state_db_path: &Path) {
    let conn = match Connection::open(thumbnails_db_path) {
        Ok(c) => c,
        Err(e) => {
            log::debug!(
                "[Migration] Could not open old thumbnails.db at {:?}: {:?} — skipping.",
                thumbnails_db_path,
                e
            );
            return;
        }
    };

    // Check whether any legacy table still exists in thumbnails.db.
    let has_legacy: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN \
             ('user_preferences','folder_locks','pinned_folders','folder_covers')",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0;

    if !has_legacy {
        return; // Already migrated or fresh install — nothing to do.
    }

    log::info!(
        "[Migration] Legacy tables detected in {:?} — migrating to {:?}",
        thumbnails_db_path,
        app_state_db_path
    );

    // ATTACH the new app_state.db.
    let attach_path = app_state_db_path.to_string_lossy().replace('\'', "''");
    if let Err(e) = conn.execute_batch(&format!("ATTACH DATABASE '{}' AS new_state", attach_path)) {
        log::error!("[Migration] Failed to ATTACH app_state.db: {:?}", e);
        return;
    }

    // Helper: check if a table exists in the old DB.
    let table_exists = |name: &str| -> bool {
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
            [name],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0)
            > 0
    };

    let mut total_migrated: usize = 0;

    // --- user_preferences ---
    if table_exists("user_preferences") {
        match conn.execute(
            "INSERT OR IGNORE INTO new_state.user_preferences (key, value) \
             SELECT key, value FROM user_preferences",
            [],
        ) {
            Ok(n) => {
                log::info!("[Migration] user_preferences: {} rows copied", n);
                total_migrated += n;
            }
            Err(e) => log::warn!("[Migration] user_preferences copy failed: {:?}", e),
        }
    }

    // --- folder_covers ---
    if table_exists("folder_covers") {
        match conn.execute(
            "INSERT OR IGNORE INTO new_state.folder_covers (folder_path, cover_path) \
             SELECT folder_path, cover_path FROM folder_covers",
            [],
        ) {
            Ok(n) => {
                log::info!("[Migration] folder_covers: {} rows copied", n);
                total_migrated += n;
            }
            Err(e) => log::warn!("[Migration] folder_covers copy failed: {:?}", e),
        }
    }

    // --- folder_locks (handles both old schema with search_query and new schema) ---
    if table_exists("folder_locks") {
        match conn.execute(
            "INSERT OR IGNORE INTO new_state.folder_locks \
             (path, view_mode, sort_mode, sort_descending, folders_position) \
             SELECT path, view_mode, sort_mode, sort_descending, folders_position \
             FROM folder_locks",
            [],
        ) {
            Ok(n) => {
                log::info!("[Migration] folder_locks: {} rows copied", n);
                total_migrated += n;
            }
            Err(e) => log::warn!("[Migration] folder_locks copy failed: {:?}", e),
        }
    }

    // --- pinned_folders ---
    if table_exists("pinned_folders") {
        match conn.execute(
            "INSERT OR IGNORE INTO new_state.pinned_folders (path, display_name, position) \
             SELECT path, display_name, position FROM pinned_folders",
            [],
        ) {
            Ok(n) => {
                log::info!("[Migration] pinned_folders: {} rows copied", n);
                total_migrated += n;
            }
            Err(e) => log::warn!("[Migration] pinned_folders copy failed: {:?}", e),
        }
    }

    // Detach before modifying the old database.
    let _ = conn.execute_batch("DETACH DATABASE new_state");

    // Drop migrated tables from old DB.
    for table in &[
        "user_preferences",
        "folder_locks",
        "pinned_folders",
        "folder_covers",
    ] {
        if let Err(e) = conn.execute(&format!("DROP TABLE IF EXISTS {}", table), []) {
            log::warn!("[Migration] Failed to drop old table {}: {:?}", table, e);
        }
    }

    // Drop orphaned cache-index tables (DirectoryIndex now uses its own DB file).
    for table in &["directory_index", "file_index"] {
        let _ = conn.execute(&format!("DROP TABLE IF EXISTS {}", table), []);
    }

    // Reclaim space.
    if let Err(e) = conn.execute_batch("VACUUM") {
        log::debug!("[Migration] VACUUM failed (non-critical): {:?}", e);
    }

    log::info!(
        "[Migration] Complete — {} total rows migrated from thumbnails.db → app_state.db",
        total_migrated
    );
}
