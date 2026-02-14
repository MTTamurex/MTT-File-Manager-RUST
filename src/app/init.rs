//! Application initialization logic.
//!
//! This module handles the creation of the `ImageViewerApp` instance, setting up
//! asynchronous workers, channels, and loading initial state/configuration.

// use eframe::egui;
use lru::LruCache;
use std::num::NonZeroUsize;
// PERFORMANCE: FxHashSet uses faster hashing for PathBuf keys
use crate::ui::cache::FxHashSet;
use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Instant;

use crate::application::ClipboardManager;
use crate::domain::file_entry::FileEntry;
use crate::infrastructure::directory_cache::DirectoryCache;
use crate::infrastructure::directory_index::DirectoryIndex;
use crate::infrastructure::disk_cache::ThumbnailDiskCache;
use crate::infrastructure::onedrive;
use crate::infrastructure::windows as windows_infra;
// use crate::ui::cache::CacheManager;
use crate::ui::context_menu::ContextMenuState;
use crate::ui::icon_loader::IconLoader;
use crate::ui::svg_icons::SvgIconManager;
use crate::ui::theme;

use super::drive_state::DriveState;
use super::file_operation_state::FileOperationState;
use super::folder_size_state::FolderSizeState;
use super::global_search_state::GlobalSearchState;
use super::init_preferences::StartupPreferences;
use super::init_workers::{
    spawn_async_font_loader, spawn_cover_worker, spawn_disk_cache_invalidation_worker,
    spawn_file_operation_worker, spawn_folder_preview_workers, spawn_folder_size_worker,
    spawn_global_search_worker, spawn_icon_worker, spawn_incremental_gc_worker,
    spawn_metadata_worker, spawn_prefetching_workers, spawn_startup_drive_info_preload,
    PrefetchWorkerHandles,
};
use super::layout_state::LayoutState;
use super::navigation_state::NavigationState;
use super::state::{ImageViewerApp, ItemsRebuildResult, LastInput};

/// Determines the initial path based on the last saved folder
/// Returns (path, is_computer_view) - if the folder is unavailable, returns "This PC"
fn determine_initial_path(disk_cache: &ThumbnailDiskCache) -> (String, bool) {
    // Try to load last folder from database
    if let Some(last_folder) = disk_cache.get_preference("last_folder") {
        if !last_folder.is_empty() {
            // Check if path still exists and is accessible
            let path_buf = PathBuf::from(&last_folder);

            // CRITICAL FIX: Use fast_path_exists() + fast_is_dir() instead of
            // path.exists() + std::fs::read_dir(). The original calls use CreateFileW
            // and FindFirstFileW which can block for 30-60s on OneDrive cloud-only
            // folders, freezing the app at startup.
            // GetFileAttributesW reads cached attributes - no network I/O.
            if onedrive::fast_path_exists(&path_buf) && onedrive::fast_is_dir(&path_buf) {
                log::info!("[INIT] Restoring last folder: {}", last_folder);
                return (last_folder, false);
            } else {
                log::warn!(
                    "[INIT] Last folder no longer exists or not accessible: {}, using Este Computador",
                    last_folder
                );
            }
        }
    }

    // Default to "This PC" if no valid last folder
    log::info!("[INIT] No valid last folder found, starting at Este Computador");
    ("Este Computador".to_string(), true)
}

// Helper function also present in main.rs - could be moved to infrastructure if needed
// Function removed: using crate::infrastructure::windows::get_all_drives instead

impl ImageViewerApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let ctx = cc.egui_ctx.clone();

        // 1. Channels for Workers -> UI communication
        let (file_entry_sender, file_entry_receiver) = mpsc::channel::<(usize, Vec<FileEntry>)>();
        let (items_rebuild_sender, items_rebuild_receiver) = mpsc::channel::<ItemsRebuildResult>();

        // Initialize disk cache (MOVED UP for Cover Worker access)
        let cache_dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("MTT-File-Manager")
            .join("thumbnails");
        let disk_cache = Arc::new(match ThumbnailDiskCache::new(cache_dir.clone()) {
            Ok(cache) => cache,
            Err(e) => {
                log::error!(
                    "[Cache] Fatal: failed to initialize thumbnail cache at {:?}: {:?}",
                    cache_dir,
                    e
                );
                std::process::exit(1);
            }
        });
        let directory_index = match DirectoryIndex::open(&cache_dir.join("thumbnails.db")) {
            Ok(index) => Some(Arc::new(index)),
            Err(e) => {
                log::warn!("[Cache] Failed to open directory index: {:?}", e);
                None
            }
        };

        // COVER WORKER: Single worker to process folder covers
        let (cover_req_tx, cover_res_rx) = spawn_cover_worker(disk_cache.clone());
        #[cfg(feature = "notify-watcher")]
        let (fs_tx, fs_rx) = mpsc::channel();
        let (device_event_sender, device_event_receiver) = mpsc::channel();

        windows_infra::start_device_change_listener(device_event_sender, ctx.clone());

        // --- THUMBNAIL SYSTEM (OPTIMIZED WORKER POOL) ---
        let (img_tx, img_rx) = mpsc::channel();
        use crate::workers::thumbnail::PriorityThumbnailQueue;
        let thumbnail_queue = Arc::new(PriorityThumbnailQueue::new());
        let shared_gen = Arc::new(AtomicUsize::new(0));

        // Initialize OneDrive path detection
        onedrive::init_onedrive_paths();

        let directory_cache = Arc::new(DirectoryCache::new());

        let StartupPreferences {
            sort_mode,
            sort_mode_computer,
            sort_mode_normal,
            sort_descending,
            folders_position,
            thumbnail_size,
            view_mode,
            show_preview_panel,
            upload_budget_ms,
            saved_window_width,
            saved_window_height,
            saved_is_maximized,
            sidebar_left_width,
            sidebar_right_width,
            saved_media_volume,
        } = StartupPreferences::load(&disk_cache);

        // STARTUP OPTIMIZATION: Async Font Loader
        // Spawns a thread to load fonts while the app frame initializes
        let font_rx = spawn_async_font_loader();

        // Shared pending_deletions for worker cancellation
        let pending_deletions: Arc<dashmap::DashMap<PathBuf, ()>> =
            Arc::new(dashmap::DashMap::new());

        // 8 threads: optimal balance between SSD and USB HDD
        use crate::workers::thumbnail::spawn_thumbnail_workers;
        spawn_thumbnail_workers(
            thumbnail_queue.clone(),
            img_tx,
            ctx.clone(),
            shared_gen.clone(),
            disk_cache.clone(),
            pending_deletions.clone(),
        );

        // --- ASYNC ICON + METADATA WORKERS ---
        let (icon_req_tx, icon_res_rx) = spawn_icon_worker(&ctx);
        let (meta_req_tx, meta_res_rx) = spawn_metadata_worker(&ctx);

        // --- FOLDER PREVIEW WORKERS ---
        let (folder_preview_tx, folder_preview_res_rx) =
            spawn_folder_preview_workers(&ctx, disk_cache.clone());

        // --- FOLDER SIZE WORKER ---
        let (folder_size_req_tx, folder_size_res_rx, folder_size_cancel) =
            spawn_folder_size_worker(&ctx);

        let PrefetchWorkerHandles {
            prefetch_sender: prefetch_tx,
            predictive_sender: predictive_tx,
            idle_warmup_sender: idle_warmup_tx,
        } = spawn_prefetching_workers(
            directory_cache.clone(),
            thumbnail_queue.clone(),
            shared_gen.clone(),
        );

        // --- FILE OPERATION WORKER (Background Shell ops) ---
        let (file_op_tx, file_op_res_rx) = spawn_file_operation_worker();

        // --- GLOBAL SEARCH WORKER (IPC client to search service) ---
        let (global_search_tx, global_search_res_rx) = spawn_global_search_worker(&ctx);

        // --- DISK CACHE INVALIDATION WORKER (async SQLite cleanup) ---
        let disk_cache_invalidation_tx = spawn_disk_cache_invalidation_worker(disk_cache.clone());

        let disks = windows_infra::get_all_drives();
        let (drive_scan_tx, drive_scan_rx) = mpsc::channel();
        let (drive_info_tx, drive_info_rx) = mpsc::channel();

        // Initialize Audio Device (removed)

        // Determine initial path based on last saved folder
        let (initial_path, is_computer_view_initial) = determine_initial_path(&disk_cache);

        // Create tab manager with the initial path
        let mut tab_manager = if is_computer_view_initial {
            crate::tabs::TabManager::new()
        } else {
            crate::tabs::TabManager::new_at_path(&initial_path)
        };
        // Set the initial tab's view mode from saved preference
        tab_manager.active_mut().view_mode = view_mode;

        let mut app = Self {
            navigation_state: NavigationState::new(initial_path.clone(), is_computer_view_initial),
            current_folder_modified_hint: None,
            folder_modified_hints: std::collections::HashMap::new(),
            loaded_path: String::new(), // Start empty - will be set when first folder loads
            thumbnail_queue,
            image_receiver: img_rx,
            pending_thumbnails: std::collections::VecDeque::new(),
            items: Arc::new(Vec::new()),
            // Async loading
            file_entry_receiver,
            file_entry_sender,
            is_loading_folder: false,
            loading_started_at: Instant::now(),
            items_rebuild_sender,
            items_rebuild_receiver,
            items_rebuild_request_id: 0,
            // Cover Worker
            cover_worker_sender: cover_req_tx,
            cover_worker_receiver: cover_res_rx,
            scanned_folders: LruCache::new(
                NonZeroUsize::new(200).expect("scanned_folders cache size must be non-zero"),
            ),
            // audio_device, // Removed
            // Folder Preview Worker (Native Windows Shell)
            folder_preview_sender: folder_preview_tx,
            folder_preview_receiver: folder_preview_res_rx,
            // Cache Manager (unifica texture_cache, icon_cache, loading_set, etc.)
            cache_manager: crate::ui::cache::CacheManager::new(),
            // Sorting - loaded from SQLite or defaults
            sort_mode,
            sort_mode_computer,
            sort_mode_normal,
            sort_descending,
            folders_position,
            disk_cache: disk_cache.clone(),
            directory_cache: directory_cache.clone(),
            directory_index: directory_index.clone(),
            // View mode: loaded from SQLite
            view_mode,
            // Selection & Preview
            selected_file: None,
            selected_thumbnail: None,
            selected_gif: None,
            media_preview: None,
            media_preview_owner_tab_id: None,
            selected_metadata: None,
            show_preview_panel, // Loaded from SQLite
            drive_state: DriveState {
                disks,
                last_drive_refresh: Instant::now(),
                last_drive_bitmask: crate::infrastructure::windows::get_logical_drives_bitmask(),
                drive_scan_pending: false,
                drive_scan_rx,
                drive_scan_tx,
                drive_info_rx,
                drive_info_tx,
                drive_info_cache: std::collections::HashMap::new(),
            },
            thumbnail_size, // Loaded from SQLite
            selected_item: None,
            multi_selection: FxHashSet::default(),
            is_item_dragging: false,
            drag_payload_paths: Vec::new(),
            drag_source_folder: None,
            drag_target_folder: None,
            drag_hovered_folder: None,
            total_items: 0,
            // Search & Navigation (NEW)
            all_items: Vec::new(),
            search_query: String::new(),
            last_grid_cols: 1,
            generation: 0,
            current_generation: shared_gen,
            ui_ctx: ctx.clone(),
            last_items_rebuild: Instant::now(),
            pending_items_rebuild: false,
            pending_items_count: 0,
            renaming_state: None,
            focus_rename: false,

            // Drive-wide file system watcher (File Pilot optimization)
            drive_watcher:
                crate::infrastructure::drive_watcher_integration::DriveWatcherManager::new(),

            #[cfg(feature = "notify-watcher")]
            watcher: None,
            #[cfg(feature = "notify-watcher")]
            fs_event_receiver: fs_rx,
            #[cfg(feature = "notify-watcher")]
            fs_event_sender: fs_tx,
            device_event_receiver,
            last_auto_reload: Instant::now(),
            pending_auto_reload: false,
            skip_next_auto_reload: false,

            // CLIPBOARD
            clipboard: ClipboardManager::new(),

            // CONTEXT MENU STATE
            context_menu: ContextMenuState::new(),

            // PERSISTENT ICON LOADER
            item_icon_loader: IconLoader::new(),

            // ASYNC ICON WORKER
            icon_req_sender: icon_req_tx,
            icon_res_receiver: icon_res_rx,
            loading_icons: FxHashSet::default(),
            failed_icons: LruCache::new(
                NonZeroUsize::new(1000).expect("failed_icons cache size must be non-zero"),
            ),

            // NOTIFICATION SYSTEM
            notifications: crate::application::NotificationManager::new(),

            // OPTIMIZED GIF MANAGER
            gif_manager: crate::ui::components::gif_manager::GifManager::new(ctx.clone()),

            // ONEDRIVE SIDEBAR SHORTCUT
            onedrive_path: std::env::var("OneDrive")
                .ok()
                .or_else(|| std::env::var("OneDriveConsumer").ok())
                .or_else(|| std::env::var("OneDriveCommercial").ok()),
            onedrive_icon: None, // Will be loaded lazily on first sidebar render

            // NAVIGATION / ADDRESS BAR
            is_address_editing: false,

            // SCROLL TO SELECTED (for keyboard navigation)
            scroll_to_selected: false,
            selection_anchor: None,
            pending_select_path: None,

            // Throttle for keyboard navigation (prevents scroll desync when holding arrow keys)
            last_keyboard_nav: Instant::now(),

            // Debounce for paste key (keys_down can fire multiple times)
            paste_key_debounce: false,

            // Native HWND (captured on first update)
            native_hwnd: None,

            // 3-stage startup counter
            startup_tick: 0,

            // STARTUP OPTIMIZATION: Async Font Loader
            font_loader_rx: Some(font_rx),

            // Window/layout persistence
            layout: LayoutState {
                saved_window_width,
                saved_window_height,
                saved_is_maximized,
                saved_is_minimized: false,
                sidebar_left_width,
                sidebar_right_width,
                list_col_name_width: disk_cache
                    .get_preference("list_col_name_width")
                    .and_then(|s| s.parse::<f32>().ok())
                    .unwrap_or(300.0),
                list_col_date_width: disk_cache
                    .get_preference("list_col_date_width")
                    .and_then(|s| s.parse::<f32>().ok())
                    .unwrap_or(170.0),
                list_col_type_width: disk_cache
                    .get_preference("list_col_type_width")
                    .and_then(|s| s.parse::<f32>().ok())
                    .unwrap_or(120.0),
                list_col_size_width: disk_cache
                    .get_preference("list_col_size_width")
                    .and_then(|s| s.parse::<f32>().ok())
                    .unwrap_or(100.0),
                list_col_onedrive_name_width: disk_cache
                    .get_preference("list_col_onedrive_name_width")
                    .and_then(|s| s.parse::<f32>().ok())
                    .unwrap_or(300.0),
                list_col_onedrive_date_width: disk_cache
                    .get_preference("list_col_onedrive_date_width")
                    .and_then(|s| s.parse::<f32>().ok())
                    .unwrap_or(170.0),
                list_col_onedrive_type_width: disk_cache
                    .get_preference("list_col_onedrive_type_width")
                    .and_then(|s| s.parse::<f32>().ok())
                    .unwrap_or(120.0),
                list_col_onedrive_size_width: disk_cache
                    .get_preference("list_col_onedrive_size_width")
                    .and_then(|s| s.parse::<f32>().ok())
                    .unwrap_or(100.0),
                list_col_onedrive_status_width: disk_cache
                    .get_preference("list_col_onedrive_status_width")
                    .and_then(|s| s.parse::<f32>().ok())
                    .unwrap_or(120.0),
                list_col_computer_name_width: disk_cache
                    .get_preference("list_col_computer_name_width")
                    .and_then(|s| s.parse::<f32>().ok())
                    .unwrap_or(300.0),
                list_col_computer_total_width: disk_cache
                    .get_preference("list_col_computer_total_width")
                    .and_then(|s| s.parse::<f32>().ok())
                    .unwrap_or(120.0),
                list_col_computer_free_width: disk_cache
                    .get_preference("list_col_computer_free_width")
                    .and_then(|s| s.parse::<f32>().ok())
                    .unwrap_or(120.0),
            },

            // METADATA ASYNC
            metadata_req_sender: meta_req_tx,
            metadata_res_receiver: meta_res_rx,
            metadata_cache: LruCache::new(
                NonZeroUsize::new(theme::METADATA_CACHE_SIZE.max(1))
                    .expect("METADATA_CACHE_SIZE.max(1) must be non-zero"),
            ),
            metadata_loading: FxHashSet::default(),
            last_metadata_refresh: Instant::now(),
            last_metadata_path: None,

            // SVG ICON MANAGER - using embedded resources
            svg_icon_manager: SvgIconManager::new(),

            // LAST INPUT STATE
            last_input: LastInput::Mouse,

            // TAB SYSTEM
            tab_manager,

            // FOLDER SIZE CALCULATOR
            folder_size_state: FolderSizeState {
                req_sender: folder_size_req_tx,
                res_receiver: folder_size_res_rx,
                cancel: folder_size_cancel,
                cache: LruCache::new(
                    NonZeroUsize::new(500).expect("folder_size cache size must be non-zero"),
                ),
                loading: FxHashSet::default(),
            },

            // RECYCLE BIN CACHE
            deletion_date_cache: LruCache::new(
                NonZeroUsize::new(200).expect("deletion_date cache size must be non-zero"),
            ),

            // PERFORMANCE: Reusable buffers for grid rendering
            pending_ops: crate::ui::views::grid_view::PendingOperations::new(),
            scroll_predictor: crate::ui::views::grid_view::ScrollPredictor::new(),

            // Scroll offset for manual grid virtualization
            scroll_offset_y: 0.0,

            // PERFORMANCE: Visible item range for GPU upload prioritization
            visible_index_range: None,

            // PERFORMANCE: Cached visible paths set to avoid per-frame allocation during scroll
            visible_paths_cache: FxHashSet::default(),
            visible_range_cached: None,

            // PERFORMANCE: Scroll state tracking for adaptive GPU upload throttling
            last_scroll_time: Instant::now(),
            last_scroll_offset: 0.0,
            frame_time_avg_ms: 0.0,
            frame_time_peak_ms: 0.0,
            fps_avg: 0.0,
            upload_budget_ms,
            last_upload_budget_update: Instant::now(),
            last_memory_maintenance: Instant::now(),

            // INACTIVITY RECOVERY
            last_restore_time: Instant::now(),
            minimized_duration_secs: 0.0,

            // PREFERENCES DEBOUNCE
            preferences_dirty: false,
            preferences_last_save: Instant::now(),

            saved_media_volume,

            scroll_request: crate::app::state::ScrollRequest::None,

            // GLOBAL SEARCH
            global_search: GlobalSearchState::new(global_search_tx, global_search_res_rx),

            // FILE OPERATION WORKER/TRACKING
            file_operation_state: FileOperationState {
                file_op_sender: file_op_tx,
                file_op_res_receiver: file_op_res_rx,
                disk_cache_invalidation_sender: disk_cache_invalidation_tx,
                prefetch_sender: prefetch_tx,
                predictive_sender: predictive_tx,
                idle_warmup_sender: idle_warmup_tx,
                file_ops_in_progress: 0,
                pending_deletions,
                pending_iso_mount: None,
            },

            // BULK THUMBNAIL SCAN
            bulk_thumbnail_scanning: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            bulk_thumbnail_was_scanning: false,
            bulk_thumbnail_total: Arc::new(std::sync::atomic::AtomicUsize::new(0)),

            // Media keyboard debounce
            last_media_key_press: std::time::Instant::now(),
        };

        // Start initial folder monitoring
        app.watch_current_folder();

        // Pre-populate drive_info_cache at startup so the details panel can show
        // drive info even if the user never visits "This PC".
        let disks_snapshot: Vec<String> = app
            .drive_state
            .disks
            .iter()
            .map(|(p, _)| p.clone())
            .collect();
        spawn_startup_drive_info_preload(
            disks_snapshot,
            app.drive_state.drive_info_tx.clone(),
            ctx.clone(),
        );

        // Background Garbage Collector (incremental + idle window)
        // Avoids aggressive startup I/O and keeps cleanup bounded on HDD.
        spawn_incremental_gc_worker(app.disk_cache.clone());

        // NOTE: Shell warmup is now done in window.rs after HWND is obtained
        // Removed duplicate warmup here to avoid protection issues

        // --- PDF WEBVIEW2 WARMUP ---
        // Initializes the runtime in a background thread to reduce latency on first PDF open.
        // Completely invisible and non-blocking.
        crate::pdf_viewer::warmup();

        app
    }
}
