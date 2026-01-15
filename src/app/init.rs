//! Application initialization logic.
//!
//! This module handles the creation of the `ImageViewerApp` instance, setting up
//! asynchronous workers, channels, and loading initial state/configuration.

// use eframe::egui;
use lru::LruCache;
use std::collections::HashSet;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Instant;

use crate::application::navigation::NavigationHistory;
use crate::application::ClipboardManager;
use crate::domain::file_entry::{FileEntry, FoldersPosition, SortMode, ViewMode};
use crate::infrastructure::disk_cache::ThumbnailDiskCache;
use crate::infrastructure::onedrive;
use crate::infrastructure::windows as windows_infra;
// use crate::ui::cache::CacheManager;
use crate::ui::context_menu::ContextMenuState;
use crate::ui::icon_loader::IconLoader;
use crate::ui::svg_icons::SvgIconManager;
use crate::ui::theme;

use super::state::ImageViewerApp;

// These are referenced from main.rs and need to be accessible
const PATH_PADRAO: &str = "C:\\";

// Função auxiliar que também está em main.rs - pode ser movida para infrastructure se necessário
// Function removed: using crate::infrastructure::windows::get_all_drives instead

impl ImageViewerApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let ctx = cc.egui_ctx.clone();

        // 1. Canais para comunicação Workers → UI
        let (file_entry_sender, file_entry_receiver) = mpsc::channel::<(usize, Vec<FileEntry>)>();

        // COVER WORKER: Worker único para processar capas de pasta
        let (cover_req_tx, cover_req_rx) = mpsc::channel::<PathBuf>(); // UI → Worker
        let (cover_res_tx, cover_res_rx) = mpsc::channel(); // Worker → UI
        let (fs_tx, fs_rx) = mpsc::channel();
        let (device_event_sender, device_event_receiver) = mpsc::channel();

        windows_infra::start_device_change_listener(device_event_sender, ctx.clone());

        // Spawna WORKER THREAD: fica em loop processando fila
        std::thread::spawn(move || {
            // Loop infinito: consome requisições da fila
            while let Ok(folder_path) = cover_req_rx.recv() {
                // Executa busca (imagem ou vídeo) usando detecção dinâmica baseado no Registro do Windows
                let cover = windows_infra::find_folder_preview_item(&folder_path);

                // Devolve resultado para UI thread
                let _ = cover_res_tx.send((folder_path, cover));
            }
        });

        // --- SISTEMA DE THUMBNAILS (WORKER POOL OTIMIZADO) ---
        let (img_tx, img_rx) = mpsc::channel();
        let (req_tx, req_rx) = mpsc::channel::<(PathBuf, usize)>();
        let shared_req_rx = Arc::new(std::sync::Mutex::new(req_rx));
        let shared_gen = Arc::new(AtomicUsize::new(0));

        // Initialize OneDrive path detection
        onedrive::init_onedrive_paths();

        // Initialize disk cache
        let cache_dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("MTT-File-Manager")
            .join("thumbnails");
        let disk_cache = Arc::new(ThumbnailDiskCache::new(cache_dir));

        // Load Preferences from SQLite
        let sort_mode = disk_cache
            .get_preference("sort_mode")
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

        // 8 threads: equilíbrio ideal entre SSD e HDD USB
        use crate::workers::thumbnail_worker::spawn_thumbnail_workers;
        spawn_thumbnail_workers(
            shared_req_rx,
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

            while let Ok(path) = icon_req_rx.recv() {
                if let Ok((pixels, width, height)) =
                    extract_file_icon_by_path(&path, IconSize::Large)
                {
                    let _ = icon_res_tx.send((path, pixels, width, height));
                    icon_ctx.request_repaint();
                }
            }
        });

        // --- METADATA WORKER (assíncrono para HDD lentos) ---
        let (meta_req_tx, meta_req_rx) = mpsc::channel::<(PathBuf, u64)>();
        let (meta_res_tx, meta_res_rx) = mpsc::channel();
        let meta_ctx = ctx.clone();

        std::thread::spawn(move || {
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

        let disks = windows_infra::get_all_drives();

        // Initialize Audio Device (Removed: using WebView2)

        let mut app = Self {
            current_path: PATH_PADRAO.to_string(),
            thumbnail_req_sender: req_tx,
            image_receiver: img_rx,
            items: Arc::new(Vec::new()),
            // Async loading
            file_entry_receiver,
            file_entry_sender,
            is_loading_folder: false,
            // Cover Worker
            cover_worker_sender: cover_req_tx,
            cover_worker_receiver: cover_res_rx,
            scanned_folders: HashSet::new(),
            // audio_device, // Removed
            // Folder Preview Worker (Native Windows Shell)
            folder_preview_sender: folder_preview_tx,
            folder_preview_receiver: folder_preview_res_rx,
            // Cache Manager (unifica texture_cache, icon_cache, loading_set, etc.)
            cache_manager: crate::ui::cache::CacheManager::new(),
            // Sorting - carregado do SQLite ou defaults
            sort_mode,
            sort_descending,
            folders_position,
            disk_cache: disk_cache.clone(),
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
            is_computer_view: false,
            is_recycle_bin_view: false,
            navigation: NavigationHistory::new(PATH_PADRAO.to_string()),
            path_input: PATH_PADRAO.to_string(),
            disks,
            last_drive_refresh: Instant::now(),
            thumbnail_size, // Loaded from SQLite
            selected_item: None,
            total_items: 0,
            // Search & Navigation (NEW)
            all_items: Vec::new(),
            search_query: String::new(),
            last_grid_cols: 1,
            generation: 0,
            current_generation: shared_gen,
            ui_ctx: ctx,
            renaming_state: None,
            focus_rename: false,

            watcher: None,
            fs_event_receiver: fs_rx,
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
            loading_icons: HashSet::new(),

            // NOTIFICATION SYSTEM
            notifications: crate::application::NotificationManager::new(),

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

            // Throttle for keyboard navigation (prevents scroll desync when holding arrow keys)
            last_keyboard_nav: Instant::now(),

            // Debounce for paste key (keys_down can fire multiple times)
            paste_key_debounce: false,

            // HWND nativo (capturado na primeira atualização)
            native_hwnd: None,

            // 3-stage startup counter
            startup_tick: 0,

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
            metadata_loading: HashSet::new(),

            // SVG ICON MANAGER
            svg_icon_manager: SvgIconManager::new(PathBuf::from("assets/icons")),

            // TAB SYSTEM
            tab_manager: crate::tabs::TabManager::new(),

            // FOLDER SIZE CALCULATOR
            folder_size_req_sender: folder_size_req_tx,
            folder_size_res_receiver: folder_size_res_rx,
            folder_size_cache: std::collections::HashMap::new(),
            folder_size_loading: HashSet::new(),

            // RECYCLE BIN CACHE
            deletion_date_cache: LruCache::new(NonZeroUsize::new(200).unwrap()),
            
            // PERFORMANCE: Reusable buffers for grid rendering
            pending_ops: crate::ui::views::grid_view::PendingOperations::new(),
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

        app
    }
}
