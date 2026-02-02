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

use crate::application::navigation::NavigationHistory;
use crate::application::ClipboardManager;
use crate::domain::file_entry::{FileEntry, FoldersPosition, SortMode, ViewMode};
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

use super::state::{ImageViewerApp, ItemsRebuildResult, LastInput};

/// Determina o path inicial baseado na última pasta salva
/// Retorna (path, is_computer_view) - se a pasta não estiver disponível, retorna "Este Computador"
fn determine_initial_path(disk_cache: &ThumbnailDiskCache) -> (String, bool) {
    // Try to load last folder from database
    if let Some(last_folder) = disk_cache.get_preference("last_folder") {
        if !last_folder.is_empty() {
            // Check if path still exists and is accessible
            let path_buf = PathBuf::from(&last_folder);
            
            // Validate path exists
            if path_buf.exists() {
                // Additional check: verify we can read the directory
                match std::fs::read_dir(&path_buf) {
                    Ok(_) => {
                        eprintln!("[INIT] Restoring last folder: {}", last_folder);
                        return (last_folder, false);
                    }
                    Err(e) => {
                        eprintln!(
                            "[INIT] Last folder exists but not accessible ({}), using Este Computador",
                            e
                        );
                    }
                }
            } else {
                eprintln!("[INIT] Last folder no longer exists: {}, using Este Computador", last_folder);
            }
        }
    }
    
    // Default to "Este Computador" if no valid last folder
    eprintln!("[INIT] No valid last folder found, starting at Este Computador");
    ("Este Computador".to_string(), true)
}

// Função auxiliar que também está em main.rs - pode ser movida para infrastructure se necessário
// Function removed: using crate::infrastructure::windows::get_all_drives instead

impl ImageViewerApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let ctx = cc.egui_ctx.clone();

        // 1. Canais para comunicação Workers → UI
        let (file_entry_sender, file_entry_receiver) = mpsc::channel::<(usize, Vec<FileEntry>)>();
        let (items_rebuild_sender, items_rebuild_receiver) =
            mpsc::channel::<ItemsRebuildResult>();

        // Initialize disk cache (MOVED UP for Cover Worker access)
        let cache_dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("MTT-File-Manager")
            .join("thumbnails");
        let disk_cache = Arc::new(ThumbnailDiskCache::new(cache_dir.clone()));
        let directory_index = match DirectoryIndex::open(&cache_dir.join("thumbnails.db")) {
            Ok(index) => Some(Arc::new(index)),
            Err(e) => {
                eprintln!("[Cache] Warning: Failed to open directory index: {:?}", e);
                None
            }
        };

        // COVER WORKER: Worker único para processar capas de pasta
        let (cover_req_tx, cover_req_rx) = mpsc::channel::<PathBuf>(); // UI → Worker
        let (cover_res_tx, cover_res_rx) = mpsc::channel(); // Worker → UI
        #[cfg(feature = "notify-watcher")]
        let (fs_tx, fs_rx) = mpsc::channel();
        let (device_event_sender, device_event_receiver) = mpsc::channel();

        windows_infra::start_device_change_listener(device_event_sender, ctx.clone());

        let cover_worker_cache = disk_cache.clone();
        // Spawna WORKER THREAD: fica em loop processando fila
        std::thread::spawn(move || {
            // PERFORMANCE: Set background priority to minimize HDD contention with video playback
            // This worker scans folders to find first image - low priority I/O
            crate::infrastructure::io_priority::set_thread_priority(
                crate::infrastructure::io_priority::IOPriority::Background,
            );

            // Loop infinito: consome requisições da fila
            while let Ok(folder_path) = cover_req_rx.recv() {
                // Executa busca (imagem ou vídeo) usando detecção dinâmica baseado no Registro do Windows
                let cover = windows_infra::find_folder_preview_item(&folder_path);

                // SAVE TO DB IN WORKER THREAD (Avoids Main Thread Lock Contention)
                if let Some(c) = &cover {
                    cover_worker_cache.set_folder_cover(&folder_path, c);
                }

                // Devolve resultado para UI thread
                let _ = cover_res_tx.send((folder_path, cover));
            }
        });

        // --- SISTEMA DE THUMBNAILS (WORKER POOL OTIMIZADO) ---
        let (img_tx, img_rx) = mpsc::channel();
        use crate::workers::thumbnail_worker::PriorityThumbnailQueue;
        let thumbnail_queue = Arc::new(PriorityThumbnailQueue::new());
        let shared_gen = Arc::new(AtomicUsize::new(0));

        // Initialize OneDrive path detection
        onedrive::init_onedrive_paths();

        let directory_cache = Arc::new(DirectoryCache::new());

        // Load Preferences from SQLite
        let sort_mode = disk_cache
            .get_preference("sort_mode")
            .map(|s| match s.as_str() {
                "date" => SortMode::Date,
                "size" => SortMode::Size,
                "type" => SortMode::Type,
                "drive_total" => SortMode::DriveTotalSpace,
                "drive_free" => SortMode::DriveFreeSpace,
                _ => SortMode::Name,
            })
            .unwrap_or(SortMode::Name);

        let sort_mode_computer = disk_cache
            .get_preference("sort_mode_computer")
            .map(|s| match s.as_str() {
                "drive_total" => SortMode::DriveTotalSpace,
                "drive_free" => SortMode::DriveFreeSpace,
                _ => SortMode::Name,
            })
            .unwrap_or(SortMode::Name);

        let sort_mode_normal = disk_cache
            .get_preference("sort_mode_normal")
            .map(|s| match s.as_str() {
                "date" => SortMode::Date,
                "size" => SortMode::Size,
                "type" => SortMode::Type,
                _ => SortMode::Name,
            })
            .unwrap_or(SortMode::Name);

        let sort_descending = disk_cache
            .get_preference("sort_descending")
            .map(|s| s == "true")
            .unwrap_or(false);

        // STARTUP OPTIMIZATION: Async Font Loader
        // Spawns a thread to load fonts while the app frame initializes
        let (font_tx, font_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut fonts = eframe::egui::FontDefinitions::default();
            let mut loaded_fonts = Vec::new();

            // 1. Segoe UI (fonte principal)
            let segoe_path = std::path::PathBuf::from("C:\\Windows\\Fonts\\segoeui.ttf");
            if let Ok(font_data) = std::fs::read(&segoe_path) {
                fonts.font_data.insert(
                    "segoe_ui".to_owned(),
                    std::sync::Arc::new(eframe::egui::FontData::from_owned(font_data)),
                );
                loaded_fonts.push("segoe_ui".to_owned());
            }

            // 2. Segoe UI Symbol (fallback 1 - símbolos)
            let symbol_path = std::path::PathBuf::from("C:\\Windows\\Fonts\\seguisym.ttf");
            if let Ok(font_data) = std::fs::read(&symbol_path) {
                fonts.font_data.insert(
                    "segoe_ui_symbol".to_owned(),
                    std::sync::Arc::new(eframe::egui::FontData::from_owned(font_data)),
                );
                loaded_fonts.push("segoe_ui_symbol".to_owned());
            }

            // 3. Arial Unicode MS (fallback 2 - se disponível)
            // ESTE ARQUIVO É GRANDE (~22MB) - O carregamento síncrono trava o startup
            let arial_path = std::path::PathBuf::from("C:\\Windows\\Fonts\\ARIALUNI.TTF");
            if let Ok(font_data) = std::fs::read(&arial_path) {
                fonts.font_data.insert(
                    "arial_unicode".to_owned(),
                    std::sync::Arc::new(eframe::egui::FontData::from_owned(font_data)),
                );
                loaded_fonts.push("arial_unicode".to_owned());
            }

            // 4. Remix Icon (Fonte de Ícones dedicada) - Embarcada no executável
            {
                let data = crate::embedded_assets::REMIXICON_TTF.to_vec();
                fonts.font_data.insert(
                    "remix_icon".to_owned(),
                    std::sync::Arc::new(eframe::egui::FontData::from_owned(data)),
                );
                fonts.families.insert(
                    eframe::egui::FontFamily::Name("icons".into()),
                    vec!["remix_icon".to_owned()],
                );
            }

            // Adiciona apenas fontes carregadas
            if !loaded_fonts.is_empty() {
                fonts
                    .families
                    .get_mut(&eframe::egui::FontFamily::Proportional)
                    .unwrap()
                    .extend(loaded_fonts.clone());

                fonts
                    .families
                    .get_mut(&eframe::egui::FontFamily::Monospace)
                    .unwrap()
                    .extend(loaded_fonts.clone());
            }

            let _ = font_tx.send(fonts);
        });

        let folders_position = disk_cache
            .get_preference("folders_position")
            .map(|s| match s.as_str() {
                "last" => FoldersPosition::Last,
                "mixed" => FoldersPosition::Mixed,
                _ => FoldersPosition::First,
            })
            .unwrap_or(FoldersPosition::First);

        // Load UI preferences from SQLite
        let thumbnail_size = disk_cache
            .get_preference("thumbnail_size")
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(theme::THUMBNAIL_DEFAULT)
            .clamp(theme::THUMBNAIL_MIN, theme::THUMBNAIL_MAX); // Ensure valid range

        let view_mode = disk_cache
            .get_preference("view_mode")
            .map(|s| match s.as_str() {
                "list" => ViewMode::List,
                _ => ViewMode::Grid,
            })
            .unwrap_or(ViewMode::Grid);

        let show_preview_panel = disk_cache
            .get_preference("show_preview_panel")
            .map(|s| s != "false")
            .unwrap_or(true);

        let upload_budget_ms = disk_cache
            .get_preference("upload_budget_ms")
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(6.0)
            .clamp(2.0, 10.0);

        // Load window state from SQLite
        let saved_window_width = disk_cache
            .get_preference("window_width")
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(1280.0);
        let saved_window_height = disk_cache
            .get_preference("window_height")
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(720.0);
        let saved_is_maximized = disk_cache
            .get_preference("window_is_maximized")
            .map(|s| s == "true")
            .unwrap_or(true); // Default to maximized

        // Load sidebar widths from SQLite
        let sidebar_left_raw = disk_cache.get_preference("sidebar_left_width");
        let sidebar_right_raw = disk_cache.get_preference("sidebar_right_width");

        eprintln!(
            "[INIT] Raw sidebar values from DB: L={:?}, R={:?}",
            sidebar_left_raw, sidebar_right_raw
        );

        let sidebar_left_width = sidebar_left_raw
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(200.0);
        let sidebar_right_width = sidebar_right_raw
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(300.0);

        eprintln!(
            "[INIT] Parsed sidebar widths: L={}, R={}",
            sidebar_left_width, sidebar_right_width
        );

        // Load media player volume from SQLite
        let saved_media_volume = disk_cache
            .get_preference("media_volume")
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(1.0)
            .clamp(0.0, 1.0);

        // 8 threads: equilíbrio ideal entre SSD e HDD USB
        use crate::workers::thumbnail_worker::spawn_thumbnail_workers;
        spawn_thumbnail_workers(
            thumbnail_queue.clone(),
            img_tx,
            ctx.clone(),
            shared_gen.clone(),
            disk_cache.clone(),
        );

        // --- ASYNC ICON WORKER (single thread, evita I/O bloqueante) ---
        let (icon_req_tx, icon_req_rx) = mpsc::channel::<PathBuf>();
        let (icon_res_tx, icon_res_rx) = mpsc::channel::<(PathBuf, Vec<u8>, u32, u32)>();
        let icon_ctx = ctx.clone();

        std::thread::spawn(move || {
            use crate::domain::file_entry::IconSize;
            use crate::infrastructure::windows::extract_file_icon_by_path;
            use windows::Win32::System::Com::{
                CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED,
            };

            // Initialize COM for this thread (multithreaded like other workers)
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            }

            // PERFORMANCE: Set background priority to minimize HDD contention with video playback
            crate::infrastructure::io_priority::set_thread_priority(
                crate::infrastructure::io_priority::IOPriority::Background,
            );

            while let Ok(path) = icon_req_rx.recv() {
                // Use IconSize::Jumbo (256x256) for high-quality icons
                // IShellItemImageFactory properly extracts embedded icons from .exe/.lnk files
                match extract_file_icon_by_path(&path, IconSize::Jumbo) {
                    Ok((pixels, width, height)) => {
                        let _ = icon_res_tx.send((path, pixels, width, height));
                    }
                    Err(_) => {
                        // Send empty data to signal failure - this clears loading_icons
                        // so the UI can show a fallback icon
                        let _ = icon_res_tx.send((path, Vec::new(), 0, 0));
                    }
                }
                icon_ctx.request_repaint();
            }

            unsafe {
                CoUninitialize();
            }
        });

        // --- METADATA WORKER (assíncrono para HDD lentos) ---
        let (meta_req_tx, meta_req_rx) = mpsc::channel::<(PathBuf, u64)>();
        let (meta_res_tx, meta_res_rx) = mpsc::channel();
        let meta_ctx = ctx.clone();

        std::thread::spawn(move || {
            // PERFORMANCE: Set background priority to minimize HDD contention with video playback
            crate::infrastructure::io_priority::set_thread_priority(
                crate::infrastructure::io_priority::IOPriority::Background,
            );

            while let Ok((path, mtime)) = meta_req_rx.recv() {
                let meta = windows_infra::extract_media_metadata(&path);
                let _ = meta_res_tx.send((path, mtime, meta));
                meta_ctx.request_repaint();
            }
        });

        // --- FOLDER PREVIEW WORKER (Native Windows Shell sandwich effect) ---
        let (folder_preview_tx, folder_preview_rx_thread) = mpsc::channel::<PathBuf>();
        let (folder_preview_res_tx, folder_preview_res_rx) = mpsc::channel();
        let folder_preview_rx = Arc::new(std::sync::Mutex::new(folder_preview_rx_thread));
        {
            use crate::workers::folder_preview_worker::spawn_folder_preview_worker;
            spawn_folder_preview_worker(folder_preview_rx, folder_preview_res_tx, ctx.clone());
        }

        // --- FOLDER SIZE WORKER (async for details panel) ---
        let (folder_size_req_tx, folder_size_req_rx) = mpsc::channel::<PathBuf>();
        let (folder_size_res_tx, folder_size_res_rx) = mpsc::channel();
        let folder_size_ctx = ctx.clone();

        std::thread::spawn(move || {
            // PERFORMANCE: Set background priority to minimize HDD contention with video playback
            // This worker is especially heavy - walks entire directory trees
            crate::infrastructure::io_priority::set_thread_priority(
                crate::infrastructure::io_priority::IOPriority::Background,
            );

            while let Ok(folder_path) = folder_size_req_rx.recv() {
                // Calculate folder size recursively using walkdir
                let mut total_size: u64 = 0;
                for entry in walkdir::WalkDir::new(&folder_path)
                    .into_iter()
                    .filter_map(|e| e.ok())
                    .filter(|e| e.file_type().is_file())
                {
                    if let Ok(meta) = entry.metadata() {
                        total_size += meta.len();
                    }
                }
                let _ = folder_size_res_tx.send((folder_path, total_size));
                folder_size_ctx.request_repaint();
            }
        });

        let (prefetch_tx, prefetch_rx) = mpsc::channel();
        crate::workers::prefetch_worker::spawn_prefetch_worker(
            prefetch_rx,
            directory_cache.clone(),
        );

        let (predictive_tx, predictive_rx) = mpsc::channel();
        crate::workers::predictive_prefetch::spawn_predictive_prefetcher(
            predictive_rx,
            directory_cache.clone(),
            directory_index.clone(),
        );

        let (idle_warmup_tx, idle_warmup_rx) = mpsc::channel();
        crate::workers::idle_warmup::spawn_idle_warmup_worker(
            idle_warmup_rx,
            thumbnail_queue.clone(),
            directory_cache.clone(),
            shared_gen.clone(),
            prefetch_tx.clone(),
        );

        // --- FILE OPERATION WORKER (Background Shell ops) ---
        let (file_op_tx, file_op_rx) = mpsc::channel();
        let (file_op_res_tx, file_op_res_rx) = mpsc::channel();
        crate::workers::file_operation_worker::start_file_operation_worker(
            file_op_rx,
            file_op_res_tx,
        );

        let disks = windows_infra::get_all_drives();

        // Initialize Audio Device (removed)

        // Determine initial path based on last saved folder
        let (initial_path, is_computer_view_initial) = determine_initial_path(&disk_cache);
        
        // Create tab manager with the initial path
        let tab_manager = if is_computer_view_initial {
            crate::tabs::TabManager::new()
        } else {
            crate::tabs::TabManager::new_at_path(&initial_path)
        };

        let mut app = Self {
            current_path: initial_path.clone(),
            loaded_path: String::new(), // Start empty - will be set when first folder loads
            thumbnail_queue,
            image_receiver: img_rx,
            pending_thumbnails: std::collections::VecDeque::new(),
            items: Arc::new(Vec::new()),
            // Async loading
            file_entry_receiver,
            file_entry_sender,
            is_loading_folder: false,
            items_rebuild_sender,
            items_rebuild_receiver,
            items_rebuild_request_id: 0,
            // Cover Worker
            cover_worker_sender: cover_req_tx,
            cover_worker_receiver: cover_res_rx,
            scanned_folders: FxHashSet::default(),
            // audio_device, // Removed
            // Folder Preview Worker (Native Windows Shell)
            folder_preview_sender: folder_preview_tx,
            folder_preview_receiver: folder_preview_res_rx,
            // Cache Manager (unifica texture_cache, icon_cache, loading_set, etc.)
            cache_manager: crate::ui::cache::CacheManager::new(),
            // Sorting - carregado do SQLite ou defaults
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
            is_computer_view: is_computer_view_initial,
            computer_view_local_indices: Vec::new(),
            computer_view_network_indices: Vec::new(),
            is_recycle_bin_view: false,
            show_virtual_drive_settings: false,
            navigation: NavigationHistory::new(initial_path.clone()),
            path_input: initial_path.clone(),
            disks,
            last_drive_refresh: Instant::now(),
            thumbnail_size, // Loaded from SQLite
            selected_item: None,
            multi_selection: FxHashSet::default(),
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

            #[cfg(feature = "notify-watcher")]
            watcher: None,
            #[cfg(feature = "notify-watcher")]
            fs_event_receiver: fs_rx,
            #[cfg(feature = "notify-watcher")]
            fs_event_sender: fs_tx,
            device_event_receiver,
            last_auto_reload: Instant::now(),
            pending_auto_reload: false,

            // CLIPBOARD
            clipboard: ClipboardManager::new(),

            // CONTEXT MENU STATE
            context_menu: ContextMenuState::new(),

            // ICON LOADER PERSISTENTE
            item_icon_loader: IconLoader::new(),

            // ASYNC ICON WORKER
            icon_req_sender: icon_req_tx,
            icon_res_receiver: icon_res_rx,
            loading_icons: FxHashSet::default(),
            failed_icons: FxHashSet::default(),

            // NOTIFICATION SYSTEM
            notifications: crate::application::NotificationManager::new(),

            // GIF MANAGER OTIMIZADO
            gif_manager: crate::ui::components::gif_manager::GifManager::new(ctx.clone()),

            // ONEDRIVE SIDEBAR SHORTCUT
            onedrive_path: std::env::var("OneDrive")
                .ok()
                .or_else(|| std::env::var("OneDriveConsumer").ok())
                .or_else(|| std::env::var("OneDriveCommercial").ok()),
            onedrive_icon: None, // Will be loaded lazily on first sidebar render

            // NAVEGAÇÃO / ADDRESS BAR
            is_address_editing: false,

            // SCROLL TO SELECTED (para navegação por teclado)
            scroll_to_selected: false,
            selection_anchor: None,

            // Throttle for keyboard navigation (prevents scroll desync when holding arrow keys)
            last_keyboard_nav: Instant::now(),

            // Debounce for paste key (keys_down can fire multiple times)
            paste_key_debounce: false,

            // HWND nativo (capturado na primeira atualização)
            native_hwnd: None,

            // 3-stage startup counter
            startup_tick: 0,

            // STARTUP OPTIMIZATION: Async Font Loader
            font_loader_rx: Some(font_rx),

            // Window state persistence
            saved_window_width,
            saved_window_height,
            saved_is_maximized,

            // Sidebar widths persistence
            sidebar_left_width,
            sidebar_right_width,

            // METADATA ASYNC
            metadata_req_sender: meta_req_tx,
            metadata_res_receiver: meta_res_rx,
            metadata_cache: LruCache::new(NonZeroUsize::new(theme::METADATA_CACHE_SIZE).unwrap()),
            metadata_loading: FxHashSet::default(),
            last_metadata_refresh: Instant::now(),
            last_metadata_path: None,

            // SVG ICON MANAGER - usando recursos embarcados
            svg_icon_manager: SvgIconManager::new(),

            // LAST INPUT STATE
            last_input: LastInput::Mouse,

            // TAB SYSTEM
            tab_manager,

            // FOLDER SIZE CALCULATOR
            folder_size_req_sender: folder_size_req_tx,
            folder_size_res_receiver: folder_size_res_rx,
            folder_size_cache: std::collections::HashMap::new(),
            folder_size_loading: FxHashSet::default(),

            // RECYCLE BIN CACHE
            deletion_date_cache: LruCache::new(NonZeroUsize::new(200).unwrap()),

            // PERFORMANCE: Reusable buffers for grid rendering
            pending_ops: crate::ui::views::grid_view::PendingOperations::new(),
            scroll_predictor: crate::ui::views::grid_view::ScrollPredictor::new(),

            // Scroll offset for manual grid virtualization
            scroll_offset_y: 0.0,

            // PERFORMANCE: Scroll state tracking for adaptive GPU upload throttling
            last_scroll_time: Instant::now(),
            last_scroll_offset: 0.0,
            frame_time_avg_ms: 0.0,
            frame_time_peak_ms: 0.0,
            fps_avg: 0.0,
            upload_budget_ms,
            last_upload_budget_update: Instant::now(),

            saved_media_volume,

            scroll_request: crate::app::state::ScrollRequest::None,

            // FILE OPERATION WORKER
            file_op_sender: file_op_tx,
            file_op_res_receiver: file_op_res_rx,
            prefetch_sender: prefetch_tx,
            predictive_sender: predictive_tx,
            idle_warmup_sender: idle_warmup_tx,

            // ISO MOUNTING
            pending_iso_mount: None,

            // Media keyboard debounce
            last_media_key_press: std::time::Instant::now(),

            // List view column widths (resizable) - Regular view
            list_col_name_width: disk_cache.get_preference("list_col_name_width").and_then(|s| s.parse::<f32>().ok()).unwrap_or(300.0),
            list_col_date_width: disk_cache.get_preference("list_col_date_width").and_then(|s| s.parse::<f32>().ok()).unwrap_or(170.0),
            list_col_type_width: disk_cache.get_preference("list_col_type_width").and_then(|s| s.parse::<f32>().ok()).unwrap_or(120.0),
            list_col_size_width: disk_cache.get_preference("list_col_size_width").and_then(|s| s.parse::<f32>().ok()).unwrap_or(100.0),
            // List view column widths - OneDrive view
            list_col_onedrive_name_width: disk_cache.get_preference("list_col_onedrive_name_width").and_then(|s| s.parse::<f32>().ok()).unwrap_or(300.0),
            list_col_onedrive_date_width: disk_cache.get_preference("list_col_onedrive_date_width").and_then(|s| s.parse::<f32>().ok()).unwrap_or(170.0),
            list_col_onedrive_type_width: disk_cache.get_preference("list_col_onedrive_type_width").and_then(|s| s.parse::<f32>().ok()).unwrap_or(120.0),
            list_col_onedrive_size_width: disk_cache.get_preference("list_col_onedrive_size_width").and_then(|s| s.parse::<f32>().ok()).unwrap_or(100.0),
            list_col_onedrive_status_width: disk_cache.get_preference("list_col_onedrive_status_width").and_then(|s| s.parse::<f32>().ok()).unwrap_or(120.0),
            // List view column widths - Computer view
            list_col_computer_name_width: disk_cache.get_preference("list_col_computer_name_width").and_then(|s| s.parse::<f32>().ok()).unwrap_or(300.0),
            list_col_computer_total_width: disk_cache.get_preference("list_col_computer_total_width").and_then(|s| s.parse::<f32>().ok()).unwrap_or(120.0),
            list_col_computer_free_width: disk_cache.get_preference("list_col_computer_free_width").and_then(|s| s.parse::<f32>().ok()).unwrap_or(120.0),
        };

        // Inicia monitoramento inicial
        app.watch_current_folder();

        // Garbage Collector em background (não bloqueia a UI)
        // Delay de 3s para permitir que a UI carregue primeiro
        let gc_cache = app.disk_cache.clone();
        std::thread::spawn(move || {
            // Aguarda a UI carregar antes de iniciar o GC
            std::thread::sleep(std::time::Duration::from_secs(3));

            let removed = gc_cache.garbage_collect();
            if removed > 0 {
                eprintln!("[GC] Removed {} orphaned cache entries", removed);
            }
        });

        // NOTE: Shell warmup is now done in window.rs after HWND is obtained
        // Removed duplicate warmup here to avoid protection issues

        // --- PDF WEBVIEW2 WARMUP ---
        // Initializes the runtime in a background thread to reduce latency on first PDF open.
        // Completely invisible and non-blocking.
        crate::pdf_viewer::warmup();

        app
    }
}
