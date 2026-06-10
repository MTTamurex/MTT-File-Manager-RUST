//! Application initialization logic.
//!
//! This module handles the creation of the `ImageViewerApp` instance, setting up
//! asynchronous workers, channels, and loading initial state/configuration.

// use eframe::egui;
use eframe::egui;
use lru::LruCache;
use std::num::NonZeroUsize;
// PERFORMANCE: FxHashSet uses faster hashing for PathBuf keys
use crate::domain::special_paths::{is_virtual_path, COMPUTER_VIEW_ID};
use crate::ui::cache::FxHashSet;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

use crate::application::ClipboardManager;
use crate::infrastructure::app_state_db::AppStateDb;
use crate::infrastructure::diagnostic_logger::{diag_info, diag_warn, field_label};
use crate::infrastructure::onedrive;
// use crate::ui::cache::CacheManager;
use crate::ui::context_menu::ContextMenuState;
use crate::ui::icon_loader::IconLoader;
use crate::ui::svg_icons::SvgIconManager;
use crate::ui::theme;

use super::global_search_state::GlobalSearchState;
use super::init_bootstrap::{bootstrap_app, AppBootstrap};
use super::init_post_startup::run_post_startup_jobs;
use super::init_preferences::StartupPreferences;
use super::init_state_builders::{
    build_drive_state, build_file_operation_state, build_folder_size_state, build_layout_state,
};
use super::navigation_state::NavigationState;
use super::state::{ImageViewerApp, LastInput};

/// Determines the initial path based on the last saved folder
/// Returns (path, is_computer_view) - if the folder is unavailable, returns "This PC"
fn determine_initial_path(app_state_db: &AppStateDb) -> (String, bool) {
    // Try to load last folder from database
    if let Some(last_folder) = app_state_db.get_preference("last_folder") {
        if !last_folder.is_empty() {
            // Restore "This PC" directly — no filesystem check needed.
            if last_folder == COMPUTER_VIEW_ID {
                log::info!("[INIT] Restoring last folder: This PC");
                diag_info(
                    "startup",
                    "restore_last_folder",
                    &[field_label("result", "computer_view")],
                );
                return (COMPUTER_VIEW_ID.to_string(), true);
            }

            // Check if path still exists and is accessible
            let path_buf = PathBuf::from(&last_folder);

            // CRITICAL FIX: Use fast_path_exists() + fast_is_dir() instead of
            // path.exists() + std::fs::read_dir(). The original calls use CreateFileW
            // and FindFirstFileW which can block for 30-60s on OneDrive cloud-only
            // folders, freezing the app at startup.
            // GetFileAttributesW reads cached attributes - no network I/O.
            if onedrive::fast_path_exists(&path_buf) && onedrive::fast_is_dir(&path_buf) {
                log::info!("[INIT] Restoring last folder from preferences");
                diag_info(
                    "startup",
                    "restore_last_folder",
                    &[field_label("result", "existing_directory")],
                );
                return (last_folder, false);
            } else {
                log::warn!(
                    "[INIT] Last folder from preferences no longer exists or is not accessible; using Este Computador"
                );
                diag_warn(
                    "startup",
                    "restore_last_folder",
                    &[field_label("result", "missing_or_inaccessible")],
                );
            }
        }
    }

    // Default to "This PC" if no valid last folder
    log::info!("[INIT] No valid last folder found, starting at Este Computador");
    diag_info(
        "startup",
        "restore_last_folder",
        &[field_label("result", "default_computer_view")],
    );
    (COMPUTER_VIEW_ID.to_string(), true)
}

// Helper function also present in main.rs - could be moved to infrastructure if needed
// Function removed: using crate::infrastructure::windows::get_all_drives instead

impl ImageViewerApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let ctx = cc.egui_ctx.clone();

        let AppBootstrap {
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
        } = bootstrap_app(&ctx);

        let StartupPreferences {
            sort_mode,
            sort_mode_computer,
            sort_mode_normal,
            sort_descending,
            folders_position,
            thumbnail_size,
            view_mode,
            show_left_sidebar,
            show_preview_panel,
            upload_budget_ms,
            saved_window_width,
            saved_window_height,
            saved_is_maximized,
            sidebar_left_width,
            sidebar_right_width,
            session_volume,
            show_hidden_files,
            show_recycle_bin,
            language,
            theme_mode,
            gpu_backend_preference,
            diagnostic_mode,
            diagnostic_mode_enabled_at,
            diagnostic_mode_needs_persist,
            shortcuts,
        } = startup_preferences;

        // Apply saved language preference
        rust_i18n::set_locale(&language);

        // Apply saved theme preference
        match theme_mode {
            crate::app::navigation_state::ThemeMode::Dark => ctx.set_visuals(egui::Visuals::dark()),
            crate::app::navigation_state::ThemeMode::Light => {
                ctx.set_visuals(egui::Visuals::light())
            }
        }
        crate::ui::theme::apply_scroll_style(&ctx);

        // Load folder locks from database
        let folder_locks = app_state_db.get_all_folder_locks();

        // Load Quick Access pinned folders from database
        let pinned_folders = app_state_db.get_all_pinned_folders();

        // Determine initial path based on last saved folder
        let (initial_path, is_computer_view_initial) = determine_initial_path(&app_state_db);

        // Start the dedicated shell menu worker (STA COM thread for async extraction).
        let (shell_menu_req_tx, shell_menu_res_rx) =
            crate::infrastructure::shell_menu_worker::start_shell_menu_worker();

        #[cfg(feature = "notify-watcher")]
        let (notify_watcher_setup_sender, notify_watcher_setup_receiver) =
            std::sync::mpsc::channel();

        // Create tab manager with the initial path
        let mut tab_manager = if is_computer_view_initial {
            crate::tabs::TabManager::new()
        } else {
            crate::tabs::TabManager::new_at_path(&initial_path)
        };
        // Set the initial tab's view mode from saved preference
        {
            let active = tab_manager.active_mut();
            active.view_mode = view_mode;
            active.show_left_sidebar = show_left_sidebar;
            active.show_preview_panel = show_preview_panel;
        }

        let mut app = Self {
            navigation_state: NavigationState::new(initial_path.clone(), is_computer_view_initial),
            current_folder_modified_hint: None,
            folder_modified_hints: lru::LruCache::new(std::num::NonZeroUsize::new(500).unwrap()),
            loaded_path: String::new(), // Start empty - will be set when first folder loads
            thumbnail_queue,
            image_receiver: img_rx,
            pending_thumbnails: std::collections::VecDeque::new(),
            thumbnail_eviction_skips: std::collections::HashMap::new(),
            stale_items_snapshot: None,
            items: Arc::new(Vec::new()),
            // Async loading
            file_entry_receiver,
            file_entry_sender,
            folder_load_failure_receiver,
            folder_load_failure_sender,
            is_loading_folder: false,
            loading_started_at: Instant::now(),
            items_rebuild_sender,
            items_rebuild_receiver,
            items_rebuild_request_id: 0,
            items_rebuild_in_flight: false,
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
            pending_folder_preview_replace: FxHashSet::default(),
            suppress_next_folder_preview_invalidation: FxHashSet::default(),
            // Cache Manager (unifica texture_cache, icon_cache, loading_set, etc.)
            cache_manager: crate::ui::cache::CacheManager::new_with_folder_preview_trace(
                folder_preview_trace,
            ),
            // Sorting - loaded from SQLite or defaults
            sort_mode,
            sort_mode_computer,
            sort_mode_normal,
            sort_descending,
            sort_descending_normal: sort_descending,
            folders_position,
            folders_position_normal: folders_position,
            show_hidden_files,
            view_mode_normal: view_mode,
            disk_cache: disk_cache.clone(),
            app_state_db: app_state_db.clone(),
            directory_cache: directory_cache.clone(),
            directory_dirty_registry: Arc::new(
                crate::infrastructure::directory_dirty_registry::DirectoryDirtyRegistry::new(),
            ),
            directory_index: directory_index.clone(),
            // View mode: loaded from SQLite
            view_mode,
            // Selection & Preview
            selected_file: None,
            selected_thumbnail: None,
            selected_gif: None,
            defer_preview_work_after_selection: false,
            media_preview: None,
            media_preview_owner_tab_id: None,
            video_player_process: None,
            selected_metadata: None,
            show_left_sidebar,  // Loaded from SQLite
            show_preview_panel, // Loaded from SQLite
            show_recycle_bin,   // Loaded from SQLite
            collapse_quick_access: false,
            collapse_local_disks: false,
            collapse_network_drives: false,
            drive_state: build_drive_state(
                disks,
                drive_scan_tx,
                drive_scan_rx,
                drive_info_tx,
                drive_info_rx,
            ),
            thumbnail_size, // Loaded from SQLite
            selected_item: None,
            multi_selection: FxHashSet::default(),
            rectangle_selection_state: None,
            is_item_dragging: false,
            drag_payload_paths: Vec::new(),
            drag_payload_is_single_directory: false,
            drag_source_folder: None,
            drag_target_folder: None,
            drag_hovered_folder: None,
            drag_source_cross_panel_context: false,
            drag_cross_panel_target: None,
            drag_drop_cross_panel_context: false,
            pending_drag_move_confirmation: None,
            drag_icon_cache: None,
            total_items: 0,
            // Search & Navigation (NEW)
            all_items: Arc::new(Vec::new()),
            search_query: String::new(),
            last_grid_cols: 1,
            generation: 0,
            current_generation: shared_gen,
            ui_ctx: ctx.clone(),
            last_items_rebuild: Instant::now(),
            pending_items_rebuild: false,
            pending_items_count: 0,
            pending_all_items_clear: false,
            hold_visible_items_until_load_complete: false,
            renaming_state: None,
            focus_rename: false,
            batch_rename_state: None,
            sidebar_renaming: None,
            sidebar_rename_focus: false,

            #[cfg(feature = "notify-watcher")]
            watcher: None,
            #[cfg(feature = "notify-watcher")]
            notify_watcher_setup_sender,
            #[cfg(feature = "notify-watcher")]
            notify_watcher_setup_receiver,
            #[cfg(feature = "notify-watcher")]
            notify_watcher_setup_request_id: 0,
            #[cfg(feature = "notify-watcher")]
            fs_event_receiver: fs_rx,
            #[cfg(feature = "notify-watcher")]
            fs_event_sender: fs_tx,
            device_event_receiver,
            last_auto_reload: Instant::now(),
            pending_auto_reload: false,
            skip_next_auto_reload: false,
            watcher_cooldown_until: None,
            onedrive_pin_reload_pending: Arc::new(AtomicBool::new(false)),
            watcher_fallback_polling: false,
            watcher_fallback_fs: None,
            watcher_fallback_last_probe: Instant::now(),
            watcher_fallback_signature: None,
            dual_panel_inactive_last_probe: Instant::now(),
            rdcw_unreliable_drives: std::collections::HashMap::new(),
            pending_folder_mtime_recheck: Vec::new(),
            pending_folder_cover_refresh: Vec::new(),
            last_folder_mtime_sort: Instant::now(),
            watcher_fs_probe_cache: std::collections::HashMap::new(),
            consistency_probe_tx,
            consistency_probe_rx,
            current_folder_liveness_probe_pending: None,
            current_folder_liveness_reload_if_alive: false,

            // CLIPBOARD
            clipboard: ClipboardManager::new(),

            // CONTEXT MENU STATE
            context_menu: ContextMenuState::new(),
            shell_menu_req_tx,
            shell_menu_res_rx,
            shell_menu_loading: false,
            shell_menu_request_id: 0,

            // SESSION ICON LOADER
            item_icon_loader: {
                let mut loader = IconLoader::new();
                // Pre-set custom composed folder icon (back+front+paper_sheet).
                {
                    let (ref pixels, width, height) = custom_folder_icon;
                    loader.set_folder_icon(&ctx, pixels, width, height);
                }
                // Pre-extract special folder icons (Documents, Pictures, etc.)
                // in a single background thread so they're ready on first render.
                loader.preload_special_folder_icons();
                loader
            },

            // ASYNC ICON WORKER
            icon_req_sender: icon_req_tx,
            icon_res_receiver: icon_res_rx,
            loading_icons: FxHashSet::default(),
            loading_extensions: rustc_hash::FxHashSet::default(),
            failed_icons: LruCache::new(
                NonZeroUsize::new(1000).expect("failed_icons cache size must be non-zero"),
            ),

            // NOTIFICATION SYSTEM
            notifications: crate::application::NotificationManager::new(),
            pending_shell_open_confirmation: None,

            // OPTIMIZED GIF MANAGER
            gif_manager: crate::ui::components::gif_manager::GifManager::new(ctx.clone()),

            // NAVIGATION / ADDRESS BAR
            is_address_editing: false,
            show_address_history_menu: false,

            // SCROLL TO SELECTED (for keyboard navigation)
            scroll_to_selected: false,
            selection_anchor: None,
            pending_select_path: None,

            // Throttle for keyboard navigation (prevents scroll desync when holding arrow keys)
            last_keyboard_nav: Instant::now(),

            // Debounce for paste key (keys_down can fire multiple times)
            paste_key_debounce: false,

            // Debounce for Shift+Delete key
            delete_key_debounce: false,

            // Address-bar Enter must not bubble into list/grid "open selected".
            suppress_next_enter_open: false,

            // Ctrl+L: focus the address bar on the next toolbar render.
            address_bar_focus_request: false,

            // Native HWND (captured on first update)
            native_hwnd: None,

            // Shell op proxy HWND (created alongside native_hwnd)
            shell_op_proxy_hwnd: None,

            // 3-stage startup counter
            startup_tick: 0,

            // STARTUP OPTIMIZATION: Async Font Loader
            font_loader_rx: Some(font_rx),

            // Window/layout persistence
            layout: build_layout_state(
                &app_state_db,
                saved_window_width,
                saved_window_height,
                saved_is_maximized,
                sidebar_left_width,
                sidebar_right_width,
            ),

            // Metadata worker
            metadata_req_sender: meta_req_tx,
            metadata_res_receiver: meta_res_rx,
            metadata_cache: LruCache::new(
                NonZeroUsize::new(theme::METADATA_CACHE_SIZE.max(1))
                    .expect("METADATA_CACHE_SIZE.max(1) must be non-zero"),
            ),
            metadata_loading: FxHashSet::default(),
            live_file_size_req_sender: live_size_req_tx,
            live_file_size_res_receiver: live_size_res_rx,
            live_file_size_cache: LruCache::new(
                NonZeroUsize::new(2048).expect("live file size cache size must be non-zero"),
            ),
            live_file_size_loading: FxHashSet::default(),
            last_metadata_refresh: Instant::now(),
            last_metadata_path: None,

            // SVG ICON MANAGER - using embedded resources
            svg_icon_manager: SvgIconManager::new(),

            // LAST INPUT STATE
            last_input: LastInput::Mouse,

            // TAB SYSTEM
            tab_manager,

            // DUAL PANEL (split view) — disabled by default
            dual_panel_enabled: false,
            dual_panel_active: crate::app::dual_panel::ActivePanel::Left,
            dual_panel_inactive_state: None,
            use_active_generation_for_thumbnail_requests: false,
            suppress_file_panel_keyboard: false,

            // FOLDER SIZE CALCULATOR
            folder_size_state: build_folder_size_state(
                folder_size_req_tx,
                folder_size_res_rx,
                folder_size_cancel,
                batch_size_tx,
                batch_size_rx,
                batch_size_cancel,
                batch_size_generation,
            ),

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
            last_actual_frame_ms: 0.0,
            fps_avg: 0.0,
            upload_budget_ms,
            last_upload_budget_update: Instant::now(),
            last_memory_maintenance: Instant::now(),
            last_memory_trace_log: Instant::now(),
            last_texture_cache_retune: Instant::now(),
            last_user_activity: Instant::now(),

            // INACTIVITY RECOVERY
            last_restore_time: Instant::now(),
            minimized_duration_secs: 0.0,
            restore_burst_until: None,

            // PREFERENCES DEBOUNCE
            preferences_dirty: false,
            preferences_last_save: Instant::now(),

            session_volume,

            theme_mode,

            active_gpu_backend: String::new(), // Set after construction from render_state
            gpu_backend_preference,
            diagnostic_mode,
            diagnostic_mode_enabled_at,
            shortcuts,
            shortcut_editor: crate::app::shortcuts::ShortcutEditorState::default(),

            folder_locks,
            current_folder_locked: false,

            pinned_folders,

            // SIDEBAR FOLDER TREE
            sidebar_tree: {
                let mut tree = crate::app::state::sidebar_tree_state::SidebarTreeState::new(
                    directory_cache.clone(),
                );
                tree.set_show_hidden(show_hidden_files);
                tree
            },

            scroll_request: crate::app::state::ScrollRequest::None,

            // GLOBAL SEARCH
            global_search: GlobalSearchState::new(global_search_tx, global_search_res_rx),

            // FILE OPERATION WORKER/TRACKING
            file_operation_state: build_file_operation_state(
                file_op_tx,
                file_op_res_rx,
                extraction_progress,
                extraction_cancel,
                disk_cache_invalidation_tx,
                prefetch_tx,
                idle_warmup_tx,
                pending_deletions,
            ),

            // BULK THUMBNAIL SCAN
            bulk_thumbnail_scanning,
            bulk_thumbnail_was_scanning: false,
            bulk_thumbnail_total,
            bulk_thumbnail_completed,
            bulk_thumbnail_session,
            bulk_thumbnail_progress,

            // Media keyboard debounce
            last_media_key_press: std::time::Instant::now(),

            // INACTIVITY RECOVERY: Window focus tracking
            was_focused: true,
            focus_lost_at: None,
        };

        // Spawn tooltip background worker for async metadata/thumbnail loading (P0-02/P0-03)
        app.global_search
            .spawn_tooltip_worker(disk_cache.clone(), &ctx);

        if app.diagnostic_mode && !crate::infrastructure::diagnostic_logger::is_enabled() {
            app.set_diagnostic_mode(true);
        } else if diagnostic_mode_needs_persist {
            app.save_preferences();
            app.force_save_preferences();
        }

        // Pre-set custom composed folder icon on cache_manager (used by grid/list bridges)
        {
            let (pixels, width, height) = custom_folder_icon;
            app.cache_manager
                .set_folder_icon(&ctx, &pixels, width, height);
        }

        // Apply folder lock for the initial folder (if it has one saved)
        app.apply_folder_lock_if_present();

        // Populate the modified hint for the initial folder so the preview panel
        // shows the correct "Data modificada" immediately on startup, even if the
        // folder was never visited in the previous session (e.g. pinned shortcuts).
        // This runs once at startup (not in the render loop), so it is safe.
        if !is_computer_view_initial && !is_virtual_path(&initial_path) {
            let dest = std::path::PathBuf::from(&initial_path);
            if let Ok(meta) = std::fs::metadata(&dest) {
                if let Ok(modified_time) = meta.modified() {
                    if let Ok(duration) = modified_time.duration_since(std::time::UNIX_EPOCH) {
                        let secs = duration.as_secs();
                        if secs > 0 {
                            app.current_folder_modified_hint = Some((dest, secs));
                        }
                    }
                }
            }
        }

        run_post_startup_jobs(&mut app, &ctx);
        app.log_memory_snapshot("post-init");

        // Log GPU adapter info to file for diagnostics (works without console).
        if let Some(render_state) = &cc.wgpu_render_state {
            let info = render_state.adapter.get_info();
            app.active_gpu_backend = format!("{:?}", info.backend);
            let has_console = {
                #[cfg(target_os = "windows")]
                {
                    use std::os::windows::io::AsRawHandle;
                    let h = std::io::stderr().as_raw_handle() as usize;
                    h != 0 && h != usize::MAX
                }
                #[cfg(not(target_os = "windows"))]
                {
                    true
                }
            };
            let diag = format!(
                "GPU: {} ({:?})\nBackend: {:?}\nDriver: {} {}\nHas console: {}\nExe: {:?}\nCWD: {:?}\nTimestamp: {:?}\n",
                info.name,
                info.device_type,
                info.backend,
                info.driver,
                info.driver_info,
                has_console,
                std::env::current_exe().ok(),
                std::env::current_dir().ok(),
                std::time::SystemTime::now(),
            );
            log::info!("[GPU] {}", diag.trim());
        } else {
            app.active_gpu_backend = "glow".to_string();
        }

        if app.is_opengl_backend() {
            log::info!(
                "[GPU] OpenGL backend detected — applying conservative upload throttling to prevent UI freezes (synchronous texture uploads)"
            );
        }

        app
    }
}
