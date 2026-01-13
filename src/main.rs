use eframe::egui;
use lru::LruCache;
use mtt_file_manager::infrastructure::disk_cache::ThumbnailDiskCache;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use rayon::prelude::*;
use std::cmp::Ordering;
use std::collections::HashSet;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant, UNIX_EPOCH};

// Mapeamento Remix Icon
const ICON_ARROW_LEFT: &str = "\u{EA64}"; // Seta Esq
const ICON_ARROW_RIGHT: &str = "\u{EA6E}"; // Seta Dir
const ICON_ARROW_UP: &str = "\u{EA78}"; // Seta Cima
const ICON_REFRESH: &str = "\u{F064}"; // Recarregar
const ICON_HOME: &str = "\u{EE1B}"; // Casa/PC
const ICON_GRID: &str = "\u{ED9E}"; // Grade (Nova sugestão)
const ICON_LIST: &str = "\u{EF3E}"; // Lista
const ICON_SEARCH: &str = "\u{F0D1}"; // Lupa
const ICON_FOLDER_ADD: &str = "\u{ED5A}"; // Nova Pasta (Sugestão do usuário)
const ICON_DETAILS: &str = "\u{ECEA}"; // Detalhes (file-info-line)
const ICON_FOLDER: &str = "\u{ED9F}"; // Folder (folder-line)
const ICON_FILE: &str = "\u{ECD3}"; // File (file-line)

// Import domain types
use mtt_file_manager::application::context_menu::ContextMenuState;
use mtt_file_manager::domain::file_entry::*;
use mtt_file_manager::domain::thumbnail::*;

// Import infrastructure modules
use mtt_file_manager::infrastructure::onedrive;
use mtt_file_manager::infrastructure::windows as windows_infra;

// Import UI modules
// use mtt_file_manager::ui::status_bar; // Not used directly - imported in render_status_bar call
use mtt_file_manager::ui::icon_loader::IconLoader;
use mtt_file_manager::ui::svg_icons::SvgIconManager;

use windows::{
    core::*,
    Win32::Foundation::*,
    Win32::Storage::FileSystem::*,
    Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState,
    Win32::UI::Shell::*,
    Win32::UI::WindowsAndMessaging::{FindWindowW, SendMessageW, WM_SYSCOMMAND},
    Win32::UI::Input::KeyboardAndMouse::ReleaseCapture,
    Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND},
};

// OTIMIZAÃ‡ÃƒO: Imports para Win32 FindFirst/NextFileW (metadata em UMA syscall)
use std::os::windows::ffi::OsStringExt;
use windows::Win32::Storage::FileSystem::{
    FindClose, FindFirstFileW, FindNextFileW, FILE_ATTRIBUTE_DIRECTORY, WIN32_FIND_DATAW,
};

// Import specific Windows API functions from modules
use windows_infra::{
    extract_file_icon, extract_file_icon_by_path, format_date, format_size, get_all_drives,
    open_with_shell,
};

// Caminho padrão
const PATH_PADRAO: &str = "C:\\";

// LRU cache - limita VRAM (~50-100MB)
const CACHE_SIZE: usize = 200;

// Icon cache (menor pois ícones são compartilhados por extensão)
const ICON_CACHE_SIZE: usize = 100;

const DRIVE_REFRESH_INTERVAL_MS: u64 = 350;

/// Converte string para formato Win32 (double-null terminated)
/// Requerido por APIs como SHFileOperationW
fn to_win32_path(path: &str) -> Vec<u16> {
    path.encode_utf16()
        .chain(std::iter::once(0))
        .chain(std::iter::once(0))
        .collect()
}

// Operações de Clipboard (Copiar/Recortar)
#[derive(Clone, Copy, PartialEq, Debug)]
enum ClipboardOp {
    Copy,
    Move,
}

// AplicaÃ§Ã£o principal
struct ImageViewerApp {
    current_path: String,

    // --- SISTEMA DE THUMBNAILS OTIMIZADO ---
    thumbnail_req_sender: Sender<(PathBuf, usize)>, // UI -> Worker Pool
    image_receiver: Receiver<ThumbnailData>,        // Worker Pool -> UI

    // File system
    items: Arc<Vec<FileEntry>>, // Arc para clone barato em render loops (60 FPS)

    // Async loading (evita freeze da UI ao ler metadata)
    file_entry_receiver: Receiver<(usize, Vec<FileEntry>)>,
    file_entry_sender: Sender<(usize, Vec<FileEntry>)>,
    is_loading_folder: bool,

    // COVER WORKER: Sistema de capas de pasta (Single Thread Worker)
    cover_worker_sender: Sender<PathBuf>, // UI â†’ Worker: Envia pasta para processar
    cover_worker_receiver: Receiver<(PathBuf, Option<PathBuf>)>, // Worker â†’ UI: Resultado
    scanned_folders: HashSet<PathBuf>,    // Cache: evita re-scan

    // FOLDER PREVIEW WORKER: Native Windows Shell folder previews (sandwich effect)
    folder_preview_sender: Sender<PathBuf>,
    folder_preview_receiver: Receiver<mtt_file_manager::workers::folder_preview_worker::FolderPreviewData>,

    // Cache Manager (unifica texture_cache, icon_cache, loading_set, etc.)

    cache_manager: mtt_file_manager::ui::cache::CacheManager,

    // Sorting state
    sort_mode: SortMode,
    sort_descending: bool,             // true = Z-A, Mais Novo, Maior
    folders_position: FoldersPosition, // First, Last, Mixed

    // Persistence Layer
    disk_cache: Arc<ThumbnailDiskCache>,

    // View Mode
    view_mode: ViewMode,

    // Navigation state (histÃ³rico linear)
    navigation_history: Vec<String>, // HistÃ³rico completo de paths
    history_index: usize,            // PosiÃ§Ã£o atual no histÃ³rico
    path_input: String,              // Barra de endereÃ§o editÃ¡vel

    // UI state
    disks: Vec<(String, String)>, // (path, label)
    last_drive_refresh: Instant,
    thumbnail_size: f32, // Zoom: 64-512
    selected_item: Option<usize>,
    selected_file: Option<FileEntry>,
    selected_thumbnail: Option<egui::TextureHandle>, // Persistent thumbnail for preview panel
    selected_metadata: Option<(PathBuf, windows_infra::MediaMetadata)>,
    metadata_req_sender: Sender<(PathBuf, u64)>,
    metadata_res_receiver: Receiver<(PathBuf, u64, windows_infra::MediaMetadata)>,
    metadata_cache: LruCache<PathBuf, (u64, windows_infra::MediaMetadata)>,
    metadata_loading: HashSet<PathBuf>,
    show_preview_panel: bool,
    is_computer_view: bool, // Se estamos na view "Este Computador"
    is_recycle_bin_view: bool, // Se estamos na view da Lixeira

    total_items: usize,

    // Search & Navigation (NEW)
    all_items: Vec<FileEntry>,            // Cache mestre para busca
    search_query: String,                 // Texto da busca
    last_grid_cols: usize,                // Memória para navegação vertical (teclado)
    generation: usize,                    // Contador local (Main Thread)
    current_generation: Arc<AtomicUsize>, // Contador compartilhado (Workers)
    ui_ctx: egui::Context,                // Referência ao contexto da UI para repaints assíncronos

    // ESTADO DE RENOMEAÇÃO
    renaming_state: Option<(usize, String)>, // (Index, Texto Editável)
    focus_rename: bool,                      // Trigger para focar no input

    // SISTEMA DE WATCHER (AUTO-REFRESH)
    watcher: Option<RecommendedWatcher>,
    fs_event_receiver: Receiver<notify::Result<notify::Event>>,
    fs_event_sender: Sender<notify::Result<notify::Event>>,
    device_event_receiver: Receiver<()>,
    last_auto_reload: Instant,
    pending_auto_reload: bool,

    // CLIPBOARD (Copiar/Recortar/Colar)
    clipboard_file: Option<PathBuf>,
    clipboard_op: Option<ClipboardOp>,

    // CONTEXT MENU STATE
    context_menu: ContextMenuState,

    // ICON LOADER PERSISTENTE (evita criar novo a cada frame)
    item_icon_loader: IconLoader,

    // ASYNC ICON WORKER (evita I/O bloqueante no render loop)
    icon_req_sender: Sender<PathBuf>, // UI ? Worker
    icon_res_receiver: Receiver<(PathBuf, Vec<u8>, u32, u32)>, // Worker ? UI
    loading_icons: HashSet<PathBuf>,  // Tracking in-progress

    // NOTIFICATION SYSTEM (toast messages)
    notifications: mtt_file_manager::application::NotificationManager,

    // ONEDRIVE SIDEBAR SHORTCUT
    onedrive_path: Option<String>, // Caminho do OneDrive (se instalado)
    onedrive_icon: Option<egui::TextureHandle>, // Ãcone nativo do OneDrive

    // NAVEGAÇÃO / ADDRESS BAR (Breadcrumbs vs Edit)
    is_address_editing: bool,

    // SCROLL TO SELECTED (para navegação por teclado)
    scroll_to_selected: bool,

    // Throttle for keyboard navigation (prevents scroll desync when holding arrow keys)
    last_keyboard_nav: Instant,

    // SVG ICON MANAGER
    svg_icon_manager: SvgIconManager,

    // Debounce for paste key (keys_down can fire multiple times)
    paste_key_debounce: bool,

    // Window handle for native shell interactions
    native_hwnd: Option<HWND>,

    // 3-stage startup: hidden -> maximize/resize -> reveal
    startup_tick: usize,

    // Window state persistence
    saved_window_width: f32,
    saved_window_height: f32,
    saved_is_maximized: bool,
    
    // Sidebar widths persistence
    sidebar_left_width: f32,
    sidebar_right_width: f32,
    
    // TAB SYSTEM
    tab_manager: mtt_file_manager::tabs::TabManager,
    
    // FOLDER SIZE CALCULATOR (async for details panel)
    folder_size_req_sender: Sender<PathBuf>,  // UI → Worker
    folder_size_res_receiver: Receiver<(PathBuf, u64)>,  // Worker → UI
    folder_size_cache: std::collections::HashMap<PathBuf, u64>,  // Calculated sizes
    folder_size_loading: HashSet<PathBuf>,  // Currently calculating
    
    // RECYCLE BIN CACHE
    deletion_date_cache: LruCache<String, String>,
}

impl ImageViewerApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let ctx = cc.egui_ctx.clone();

        // 1. Canais para comunicação Workers ? UI
        let (file_entry_sender, file_entry_receiver) = mpsc::channel::<(usize, Vec<FileEntry>)>();

        // COVER WORKER: Worker Ãºnico para processar capas de pasta
        let (cover_req_tx, cover_req_rx) = mpsc::channel::<PathBuf>(); // UI â†’ Worker
        let (cover_res_tx, cover_res_rx) = mpsc::channel(); // Worker â†’ UI
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
            .unwrap_or(128.0)
            .clamp(64.0, 512.0); // Ensure valid range

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
        
        eprintln!("[INIT] Raw sidebar values from DB: L={:?}, R={:?}", sidebar_left_raw, sidebar_right_raw);
        
        let sidebar_left_width = sidebar_left_raw
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(200.0);
        let sidebar_right_width = sidebar_right_raw
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(300.0);
        
        eprintln!("[INIT] Parsed sidebar widths: L={}, R={}", sidebar_left_width, sidebar_right_width);

        // 8 threads: equilíbrio ideal entre SSD e HDD USB
        use mtt_file_manager::workers::thumbnail_worker::spawn_thumbnail_workers;
        spawn_thumbnail_workers(
            shared_req_rx,
            img_tx,
            ctx.clone(),
            shared_gen.clone(),
            disk_cache.clone(),
        );

        // --- ASYNC ICON WORKER (single thread, evita I/O bloqueante) ---
        let (icon_req_tx, icon_req_rx) = mpsc::channel::<PathBuf>();
        let (icon_res_tx, icon_res_rx) = mpsc::channel();
        let icon_ctx = ctx.clone();

        std::thread::spawn(move || {
            use mtt_file_manager::domain::file_entry::IconSize;
            use mtt_file_manager::infrastructure::windows::extract_file_icon_by_path;

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
            use mtt_file_manager::workers::folder_preview_worker::spawn_folder_preview_worker;
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

        let disks = get_all_drives();

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
            // Folder Preview Worker (Native Windows Shell)
            folder_preview_sender: folder_preview_tx,
            folder_preview_receiver: folder_preview_res_rx,
            // Cache Manager (unifica texture_cache, icon_cache, loading_set, etc.)
            cache_manager: mtt_file_manager::ui::cache::CacheManager::new(),
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
            selected_metadata: None,
            show_preview_panel, // Loaded from SQLite
            is_computer_view: false,
            is_recycle_bin_view: false,
            // Navigation - comeÃ§a com path inicial no histÃ³rico
            navigation_history: vec![PATH_PADRAO.to_string()],
            history_index: 0,
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
            device_event_receiver: device_event_receiver,
            last_auto_reload: Instant::now(),
            pending_auto_reload: false,

            // CLIPBOARD
            clipboard_file: None,
            clipboard_op: None,

            // CONTEXT MENU STATE
            context_menu: ContextMenuState::new(),

            // ICON LOADER PERSISTENTE
            item_icon_loader: IconLoader::new(),

            // ASYNC ICON WORKER
            icon_req_sender: icon_req_tx,
            icon_res_receiver: icon_res_rx,
            loading_icons: HashSet::new(),

            // NOTIFICATION SYSTEM
            notifications: mtt_file_manager::application::NotificationManager::new(),

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
            metadata_cache: LruCache::new(NonZeroUsize::new(512).unwrap()),
            metadata_loading: HashSet::new(),

            // SVG ICON MANAGER
            svg_icon_manager: SvgIconManager::new(PathBuf::from("assets/icons")),
            
            // TAB SYSTEM
            tab_manager: mtt_file_manager::tabs::TabManager::new(),
            
            // FOLDER SIZE CALCULATOR
            folder_size_req_sender: folder_size_req_tx,
            folder_size_res_receiver: folder_size_res_rx,
            folder_size_cache: std::collections::HashMap::new(),
            folder_size_loading: HashSet::new(),
            
            // RECYCLE BIN CACHE
            deletion_date_cache: LruCache::new(NonZeroUsize::new(200).unwrap()),
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

impl ImageViewerApp {
    // Helper para botÃµes de Ã­cone da Toolbar
    fn icon_button(&mut self, ui: &mut egui::Ui, icon: &str, tooltip: &str) -> egui::Response {
        let icon_name = match icon {
            ICON_ARROW_LEFT => "nav_back",
            ICON_ARROW_RIGHT => "nav_forward",
            ICON_ARROW_UP => "nav_up",
            ICON_REFRESH => "refresh",
            ICON_HOME => "home",
            ICON_SEARCH => "search",
            ICON_FOLDER_ADD => "folder_new",
            _ => return ui.button(icon).on_hover_text(tooltip),
        };

        if icon == ICON_HOME {
            if let Some(texture) = self.cache_manager.computer_icon.as_ref() {
                let response = ui.add(
                    egui::ImageButton::new(egui::load::SizedTexture::new(
                        texture.id(),
                        egui::vec2(22.0, 22.0), // Consistent with 24px SVG visual size
                    ))
                    .frame(false)
                );
                if !tooltip.is_empty() {
                    return response.on_hover_text(tooltip);
                }
                return response;
            }
        }

        // Use slightly larger icons to improve readability of the top bar controls.
        mtt_file_manager::ui::svg_icons::icon_button(ui, &mut self.svg_icon_manager, icon_name, 24.0, tooltip)
    }

    fn delete_with_shell_for_idx(&mut self, idx: Option<usize>) {
        let target_idx = idx.or(self.selected_item);
        if let Some(idx) = target_idx {
            if let Some(item) = self.items.get(idx) {
                let path = item.path.clone();
                let path_str = path.to_string_lossy().to_string();
                let from_vec = to_win32_path(&path_str);

                let mut op = SHFILEOPSTRUCTW {
                    hwnd: HWND(std::ptr::null_mut()),
                    wFunc: FO_DELETE,
                    pFrom: PCWSTR(from_vec.as_ptr()),
                    pTo: PCWSTR(std::ptr::null()),
                    fFlags: (FOF_ALLOWUNDO | FOF_WANTNUKEWARNING).0 as u16,
                    ..Default::default()
                };

                unsafe {
                    // SAFETY: `op` is properly initialized with a double-null terminated wide string
                    // from `to_win32_path`, which is required by `SHFileOperationW`.
                    let result = SHFileOperationW(&mut op);
                    if result == 0 {
                        // Limpa cache do item deletado
                        self.disk_cache.remove_cache_for_path(&path);

                        // O watcher vai cuidar do refresh, mas podemos limpar a seleção
                        if self.selected_item == Some(idx) {
                            self.selected_item = None;
                            self.selected_file = None;
                        }
                    }
                }
            }
        }
    }

    fn restore_from_recycle_bin(&mut self, physical_path: &Path) {
        use mtt_file_manager::infrastructure::windows::recycle_bin::{restore_from_recycle_bin, enumerate_recycle_bin};
        
        // Get the original path from RecycleBinItem by re-enumerating
        // This ensures we get the correct original_path stored in the $I file
        let original_path = if let Ok(recycle_items) = enumerate_recycle_bin() {
            recycle_items
                .iter()
                .find(|item| item.physical_path == physical_path)
                .map(|item| item.original_path.clone())
        } else {
            None
        };
        
        if let Some(item) = self.items.iter().find(|i| i.path == physical_path) {
            let original_path = original_path.unwrap_or_else(|| {
                // Fallback: use Desktop if we can't find original path
                PathBuf::from("C:\\Users\\Public\\Desktop").join(item.name.clone())
            });
            
            match restore_from_recycle_bin(physical_path, &original_path) {
                Ok(_) => {
                    self.notifications.push(
                        mtt_file_manager::application::AppNotification::success(
                            format!("'{}' restaurado com sucesso", item.name),
                        ),
                    );
                    // Refresh recycle bin view
                    self.setup_recycle_bin_view();
                }
                Err(e) => {
                    self.notifications.push(
                        mtt_file_manager::application::AppNotification::error(
                            format!("Erro ao restaurar: {}", e),
                        ),
                    );
                }
            }
        }
    }

    fn delete_permanently(&mut self, physical_path: &Path) {
        use mtt_file_manager::infrastructure::windows::recycle_bin::delete_permanently;
        
        if let Some(item) = self.items.iter().find(|i| i.path == physical_path) {
            let item_name = item.name.clone();
            
            match delete_permanently(physical_path) {
                Ok(_) => {
                    self.notifications.push(
                        mtt_file_manager::application::AppNotification::success(
                            format!("'{}' excluído permanentemente", item_name),
                        ),
                    );
                    // Refresh recycle bin view
                    self.setup_recycle_bin_view();
                }
                Err(e) => {
                    self.notifications.push(
                        mtt_file_manager::application::AppNotification::error(
                            format!("Erro ao excluir: {}", e),
                        ),
                    );
                }
            }
        }
    }

    fn empty_recycle_bin(&mut self) {
        use mtt_file_manager::infrastructure::windows::recycle_bin::empty_recycle_bin;
        
        match empty_recycle_bin() {
            Ok(_) => {
                self.notifications.push(
                    mtt_file_manager::application::AppNotification::success(
                        "Lixeira esvaziada com sucesso".to_string(),
                    ),
                );
                // Refresh recycle bin view
                self.setup_recycle_bin_view();
            }
            Err(e) => {
                self.notifications.push(
                    mtt_file_manager::application::AppNotification::error(
                        format!("Erro ao esvaziar lixeira: {}", e),
                    ),
                );
            }
        }
    }

    fn show_properties_for_idx(&mut self, idx: Option<usize>) {
        let target_idx = idx.or(self.selected_item);
        if let Some(idx) = target_idx {
            if let Some(item) = self.items.get(idx) {
                let path = item.path.clone();
                // We'll use the shell properties dialog
                let _ = mtt_file_manager::infrastructure::windows::native_menu::show_properties_dialog(
                    self.native_hwnd.unwrap_or_default(),
                    &path
                );
            }
        }
    }

    fn create_new_folder(&mut self) {
        let base_path = PathBuf::from(&self.current_path);
        let mut new_folder_name = "Nova Pasta".to_string();
        let mut counter = 1;

        while base_path.join(&new_folder_name).exists() {
            counter += 1;
            new_folder_name = format!("Nova Pasta ({})", counter);
        }

        let full_path = base_path.join(&new_folder_name);

        if std::fs::create_dir(&full_path).is_ok() {
            // CRÍTICO: Para renomear imediatamente, usamos o helper from_path
            let new_item = FileEntry::from_path(full_path.clone(), true);

            self.all_items.push(new_item);
            self.filter_items();
            self.sort_items();

            // Acha o índice no vetor filtrado (items)
            if let Some(idx) = self.items.iter().position(|i| i.path == full_path) {
                self.selected_item = Some(idx);
                self.selected_file = Some(self.items[idx].clone());
                self.renaming_state = Some((idx, new_folder_name));
                self.focus_rename = true;
            }

            // Requisita load real em background para garantir sincronia com disco
            self.load_folder(false);
        }
    }

    // ===== CLIPBOARD OPERATIONS (Ctrl+C, Ctrl+X, Ctrl+V) =====

    /// Copiar: Coloca o arquivo no clipboard do Windows (CF_HDROP format)
    fn command_copy(&mut self, idx: Option<usize>) {
        eprintln!("[DEBUG] command_copy called with idx: {:?}", idx);
        let target_idx = idx.or(self.selected_item);
        if let Some(idx) = target_idx {
            if let Some(item) = self.items.get(idx) {
                // Put file in Windows clipboard using CF_HDROP format
                if let Err(e) =
                    mtt_file_manager::infrastructure::windows_clipboard::copy_files_to_clipboard(&[
                        item.path.clone(),
                    ])
                {
                    eprintln!("[CLIPBOARD ERROR] Failed to copy: {}", e);
                }
                // Also keep internal state as backup
                self.clipboard_file = Some(item.path.clone());
                self.clipboard_op = Some(ClipboardOp::Copy);
            }
        }
    }

    /// Recortar: Coloca o arquivo no clipboard do Windows com flag de MOVE
    fn command_cut(&mut self, idx: Option<usize>) {
        let target_idx = idx.or(self.selected_item);
        if let Some(idx) = target_idx {
            if let Some(item) = self.items.get(idx) {
                // Put file in Windows clipboard using CF_HDROP format with MOVE effect
                if let Err(e) =
                    mtt_file_manager::infrastructure::windows_clipboard::cut_files_to_clipboard(&[
                        item.path.clone(),
                    ])
                {
                    eprintln!("[CLIPBOARD ERROR] Failed to cut: {}", e);
                }
                // Also keep internal state as backup
                self.clipboard_file = Some(item.path.clone());
                self.clipboard_op = Some(ClipboardOp::Move);
            }
        }
    }

    /// Colar: Lê do clipboard do Windows e executa SHFileOperationW
    fn command_paste(&mut self, idx: Option<usize>) {
        eprintln!("[DEBUG] command_paste called with idx: {:?}", idx);
        use mtt_file_manager::infrastructure::windows_clipboard;

        // 1. First try to read from Windows clipboard
        let (src_paths, is_move) =
            if let Some(files) = windows_clipboard::get_files_from_clipboard() {
                let op = windows_clipboard::get_clipboard_operation();
                let is_move = matches!(op, Some(windows_clipboard::ClipboardFileOp::Move));
                (files, is_move)
            } else if let Some(path) = &self.clipboard_file {
                // Fallback to internal clipboard
                let is_move = matches!(self.clipboard_op, Some(ClipboardOp::Move));
                (vec![path.clone()], is_move)
            } else {
                // Nothing to paste
                return;
            };

        if src_paths.is_empty() {
            return;
        }

        // 2. Destination folder
        // If idx is provided and is a folder, use it as destination
        let dest_folder = if let Some(idx) = idx {
            if let Some(item) = self.items.get(idx) {
                if item.is_dir {
                    item.path.clone()
                } else {
                    PathBuf::from(&self.current_path)
                }
            } else {
                PathBuf::from(&self.current_path)
            }
        } else {
            PathBuf::from(&self.current_path)
        };

        // 3. Perform operation for each file
        for src_path in &src_paths {
            // Skip if trying to move to same folder
            if is_move && src_path.parent() == Some(&dest_folder) {
                continue;
            }

            // Prepare strings for Windows API (double-null terminated)
            let mut from_vec: Vec<u16> = src_path.to_string_lossy().encode_utf16().collect();
            from_vec.push(0);
            from_vec.push(0);

            let mut to_vec: Vec<u16> = dest_folder.to_string_lossy().encode_utf16().collect();
            to_vec.push(0);
            to_vec.push(0);

            let w_func = if is_move { FO_MOVE } else { FO_COPY };

            let mut op = SHFILEOPSTRUCTW {
                hwnd: HWND(std::ptr::null_mut()),
                wFunc: w_func,
                pFrom: PCWSTR(from_vec.as_ptr()),
                pTo: PCWSTR(to_vec.as_ptr()),
                fFlags: (FOF_ALLOWUNDO).0 as u16,
                ..Default::default()
            };

            unsafe {
                // SAFETY: from_vec and to_vec are properly double-null terminated
                let result = SHFileOperationW(&mut op);
                if result != 0 {
                    eprintln!("[PASTE ERROR] SHFileOperationW returned: {}", result);
                }
            }
        }

        // 4. If it was a Move operation, clear clipboards
        if is_move {
            self.clipboard_file = None;
            self.clipboard_op = None;
            // Note: Windows clipboard is managed by the shell, we don't clear it
        }

        // 5. Reload folder to show result
        self.load_folder(false);

        // 6. Clear context menu target
        self.context_menu.target_path = None;
    }

    // Helper para botÃµes "Toggle" (que ficam acesos se selecionados)
    fn toggle_icon_button(
        &mut self,
        ui: &mut egui::Ui,
        icon: &str,
        active: bool,
        tooltip: &str,
    ) -> egui::Response {
        let icon_name = match icon {
            ICON_GRID => "view_grid",
            ICON_LIST => "view_list",
            ICON_DETAILS => "info",
            _ => {
                let color = if active {
                    egui::Color32::from_rgb(0, 120, 215)
                } else {
                    ui.visuals().text_color()
                };
                let rich_text = egui::RichText::new(icon)
                    .family(egui::FontFamily::Name("icons".into()))
                    .size(22.0)
                    .color(color);
                return ui.add(egui::Button::new(rich_text).frame(false)).on_hover_text(tooltip);
            }
        };

        let size = 24.0;
        let color = if active {
            [0, 120, 215, 255] // Blue for active
        } else if ui.visuals().dark_mode {
            [220, 220, 220, 255]
        } else {
            [60, 60, 60, 255]
        };

        // Render at 2x resolution for HiDPI quality
        let render_size = (size * 2.0) as u32;
        
        if let Some(texture) = self.svg_icon_manager.get_icon(ui.ctx(), icon_name, render_size, color) {
            let resp = ui.add(
                egui::ImageButton::new(egui::load::SizedTexture::new(
                    texture.id(),
                    egui::vec2(size, size),  // Display at requested size
                ))
                .frame(false)
            );
            if !tooltip.is_empty() {
                resp.clone().on_hover_text(tooltip);
            }
            resp
        } else {
            ui.add(egui::Button::new("?").min_size(egui::vec2(size, size)))
        }
    }

    /// Filtra itens baseado na query de busca e reaplica ordenação
    fn filter_items(&mut self) {
        if self.search_query.is_empty() {
            self.items = Arc::new(self.all_items.clone());
        } else {
            let query = self.search_query.to_lowercase();
            self.items = Arc::new(
                self.all_items
                    .iter()
                    .filter(|item| item.name.to_lowercase().contains(&query))
                    .cloned()
                    .collect(),
            );
        }
        self.total_items = self.items.len();

        // SEMPRE ordena após filtrar para manter consistência
        self.sort_items();
    }

    /// Ordena itens baseado no modo atual e preferência de posição de pastas
    /// OTIMIZADO: Usa par_sort_by para listas >5000 itens (rayon)
    fn sort_items(&mut self) {
        // Clone interno para mutação, depois wrap em novo Arc
        let mut items_vec = (*self.items).clone();

        // Closure de comparação
        let sort_mode = self.sort_mode;
        let sort_descending = self.sort_descending;
        let folders_position = self.folders_position;

        let compare = |a: &FileEntry, b: &FileEntry| -> Ordering {
            // 1. Posição das pastas (se não for Mixed)
            if folders_position != FoldersPosition::Mixed && a.is_dir != b.is_dir {
                let folders_come_first = folders_position == FoldersPosition::First;
                return if a.is_dir {
                    if folders_come_first {
                        Ordering::Less
                    } else {
                        Ordering::Greater
                    }
                } else {
                    if folders_come_first {
                        Ordering::Greater
                    } else {
                        Ordering::Less
                    }
                };
            }

            // 2. Ordena por modo selecionado (Smart Sorting com natord)
            let ordering = match sort_mode {
                SortMode::Name => natord::compare(&a.name.to_lowercase(), &b.name.to_lowercase()),
                SortMode::Date => a.modified.cmp(&b.modified),
                SortMode::Size => a.size.cmp(&b.size),
                SortMode::Type => {
                    let ext_a = a
                        .path
                        .extension()
                        .map(|e| e.to_string_lossy().to_lowercase())
                        .unwrap_or_default();
                    let ext_b = b
                        .path
                        .extension()
                        .map(|e| e.to_string_lossy().to_lowercase())
                        .unwrap_or_default();
                    match ext_a.cmp(&ext_b) {
                        std::cmp::Ordering::Equal => {
                            natord::compare(&a.name.to_lowercase(), &b.name.to_lowercase())
                        }
                        other => other,
                    }
                }
            };

            // 3. Inverte se descending está ativo
            if sort_descending {
                ordering.reverse()
            } else {
                ordering
            }
        };

        // Threshold adaptativo: paralelo para listas grandes, sequencial para pequenas
        const PARALLEL_THRESHOLD: usize = 5000;

        if items_vec.len() > PARALLEL_THRESHOLD {
            // Paralelo: usa todos os núcleos da CPU
            items_vec.par_sort_by(compare);
        } else {
            // Sequencial: evita overhead de threads para listas pequenas
            items_vec.sort_by(compare);
        }

        self.items = Arc::new(items_vec);
    }

    /// Salva as preferências atuais no SQLite
    fn save_preferences(&self) {
        let sort_mode_str = match self.sort_mode {
            SortMode::Name => "name",
            SortMode::Date => "date",
            SortMode::Size => "size",
            SortMode::Type => "type",
        };
        self.disk_cache.set_preference("sort_mode", sort_mode_str);

        self.disk_cache.set_preference(
            "sort_descending",
            if self.sort_descending {
                "true"
            } else {
                "false"
            },
        );

        let folders_pos_str = match self.folders_position {
            FoldersPosition::First => "first",
            FoldersPosition::Last => "last",
            FoldersPosition::Mixed => "mixed",
        };
        self.disk_cache
            .set_preference("folders_position", folders_pos_str);

        // UI preferences
        self.disk_cache
            .set_preference("thumbnail_size", &self.thumbnail_size.to_string());

        let view_mode_str = match self.view_mode {
            ViewMode::Grid => "grid",
            ViewMode::List => "list",
        };
        self.disk_cache.set_preference("view_mode", view_mode_str);

        self.disk_cache.set_preference(
            "show_preview_panel",
            if self.show_preview_panel { "true" } else { "false" },
        );

        // Window state persistence
        self.disk_cache
            .set_preference("window_width", &self.saved_window_width.to_string());
        self.disk_cache
            .set_preference("window_height", &self.saved_window_height.to_string());
        self.disk_cache.set_preference(
            "window_is_maximized",
            if self.saved_is_maximized { "true" } else { "false" },
        );
        
        // Sidebar widths persistence - só salva valores válidos
        let left_to_save = self.sidebar_left_width.max(150.0);
        let right_to_save = self.sidebar_right_width.max(250.0);
        self.disk_cache
            .set_preference("sidebar_left_width", &left_to_save.to_string());
        self.disk_cache
            .set_preference("sidebar_right_width", &right_to_save.to_string());
    }

    /// Requisita scan assÃ­ncrono de uma pasta para descobrir primeira imagem.
    /// OTIMIZADO: Envia mensagem para worker Ãºnico (zero overhead de threads)
    fn request_folder_scan(&self, folder_path: PathBuf) {
        // Apenas envia para fila - worker processa em background
        let _ = self.cover_worker_sender.send(folder_path);
    }

    fn load_folder(&mut self, force_refresh: bool) {
        self.generation += 1; // Incrementa a geração local
        self.current_generation
            .store(self.generation, AtomicOrdering::Relaxed); // Sincroniza com workers

        // 1. Limpeza de Estado (UI Thread)
        if force_refresh {
            self.cache_manager.texture_cache.clear();
            self.cache_manager.folder_preview_cache.clear();
        }

        self.items = Arc::new(Vec::new()); // Novo Arc vazio (antigo é dropped automaticamente)
        self.all_items.clear(); // Limpa backup mestre também
        self.cache_manager.loading_set.clear(); // Limpa apenas requisições pendentes, mantém cache de texturas
        self.scanned_folders.clear();
        self.selected_item = None;
        self.is_loading_folder = true;
        self.total_items = 0;

        let my_gen = self.generation;
        let gen_clone = self.current_generation.clone();
        let current_path = self.current_path.clone();
        let file_entry_sender = self.file_entry_sender.clone();
        let ctx = self.ui_ctx.clone();
        let disk_cache = self.disk_cache.clone();

        // STREAMING BATCH LOADING: Envia lotes de 250 itens progressivamente
        std::thread::spawn(move || {
            let scan_start = std::time::Instant::now();
            eprintln!("[PERF] Starting folder scan: {:?}", current_path);
            // Buffer para envio em lotes
            let mut batch = Vec::with_capacity(250);

            // Normaliza o path base: drive roots precisam de trailing backslash
            // Ex: "Z:" -> "Z:\\" para que PathBuf::join funcione corretamente
            let base_path = if current_path.len() == 2 && current_path.ends_with(':') {
                format!("{}\\", current_path)
            } else {
                current_path.clone()
            };

            // Prepara busca Win32
            let search_path = if base_path.ends_with('\\') {
                format!("{}*", base_path)
            } else {
                format!("{}\\*", base_path)
            };
            let wide_path: Vec<u16> = search_path
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let mut find_data = WIN32_FIND_DATAW::default();

            // Check if we're in a OneDrive folder (for sync status)
            let is_onedrive = onedrive::is_onedrive_path(&PathBuf::from(&current_path));

            unsafe {
                // SAFETY: `wide_path` is a null-terminated UTF-16 string buffer.
                // `find_data` is a valid pointer to a `WIN32_FIND_DATAW` struct.
                // The handle returned is checked for validity and closed via `FindClose`
                // before the scope ends.
                if let Ok(handle) = FindFirstFileW(PCWSTR(wide_path.as_ptr()), &mut find_data) {
                    loop {
                        
                        // Verifica se a geração mudou -> Aborta scan antigo
                        if gen_clone.load(AtomicOrdering::Relaxed) != my_gen {
                            break;
                        }

                        let len = find_data
                            .cFileName
                            .iter()
                            .position(|&c| c == 0)
                            .unwrap_or(find_data.cFileName.len());
                        let filename = std::ffi::OsString::from_wide(&find_data.cFileName[0..len])
                            .to_string_lossy()
                            .into_owned();

                        if filename != "." && filename != ".." {
                            let attrs = find_data.dwFileAttributes;
                            let full_path = PathBuf::from(&base_path).join(&filename);

                            // PERFORMANCE: Use basic attributes from FindFirstFileW/FindNextFileW.
                            // They already contain OneDrive flags (RECALL_ON_OPEN, RECALL_ON_DATA_ACCESS, PINNED).
                            // Calling GetFileAttributesW() again is redundant and adds 2ms per file!
                            //
                            // OLD CODE (removed - was causing 98% of scan time on OneDrive):
                            // let extended_attrs = if is_onedrive {
                            //     let path_wide: Vec<u16> = full_path.to_string_lossy()...
                            //     GetFileAttributesW(...)  // ← 2ms syscall PER FILE!
                            // } else {
                            //     attrs
                            // };
                            let extended_attrs = attrs;

                            // Filtros: hidden/system files
                            let is_hidden = (extended_attrs & FILE_ATTRIBUTE_HIDDEN.0) != 0;
                            let is_system = (extended_attrs & FILE_ATTRIBUTE_SYSTEM.0) != 0;
                            let is_special = matches!(
                                filename.to_lowercase().as_str(),
                                "desktop.ini"
                                    | "thumbs.db"
                                    | "$recycle.bin"
                                    | "system volume information" // Re-adicionado "System Volume Information" para garantir compatibilidade
                            );

                            if !is_hidden && !is_system && !is_special && !filename.starts_with('.')
                            {
                                let is_dir = (extended_attrs & FILE_ATTRIBUTE_DIRECTORY.0) != 0;

                                let size = if is_dir {
                                    0
                                } else {
                                    ((find_data.nFileSizeHigh as u64) << 32)
                                        | (find_data.nFileSizeLow as u64)
                                };

                                let ft = find_data.ftLastWriteTime;
                                let windows_ticks =
                                    ((ft.dwHighDateTime as u64) << 32) | (ft.dwLowDateTime as u64);
                                let modified = if windows_ticks > 116444736000000000 {
                                    (windows_ticks - 116444736000000000) / 10_000_000
                                } else {
                                    0
                                };

                                let folder_cover = if is_dir {
                                    disk_cache.get_folder_cover(&full_path)
                                } else {
                                    None
                                };

                                // Check if file is currently open (being used)
                                let sync_status =
                                    onedrive::get_sync_status(extended_attrs, is_onedrive);

                                // DISABLED: is_file_open() is EXTREMELY slow on OneDrive (28ms per file!)
                                // It tries to open file handles which triggers sync/network checks.
                                // Windows Explorer doesn't do this - it only uses file attributes.
                                // 
                                // OLD CODE (removed for performance):
                                // if is_onedrive && !is_dir && sync_status != SyncStatus::None {
                                //     if onedrive::is_file_open(&full_path) {
                                //         sync_status = SyncStatus::Syncing;
                                //     }
                                // }

                                let entry = FileEntry {
                                    path: full_path,
                                    name: filename,
                                    is_dir,
                                    size,
                                    modified,
                                    folder_cover,
                                    drive_info: None,
                                    sync_status,
                                    deletion_date: None,
                                };

                                // Adiciona ao lote
                                batch.push(entry);

                                // SE o lote encheu (250 itens), envia e limpa
                                if batch.len() >= 250 {
                                    let _ = file_entry_sender.send((my_gen, batch.clone()));
                                    batch.clear();
                                    ctx.request_repaint(); // Acorda a UI para mostrar progresso
                                }
                            }
                        }
                        
                        if FindNextFileW(handle, &mut find_data).is_err() {
                            break;
                        }
                    }
                    let _ = FindClose(handle);
                }
            }

            // Envia o restante (último lote) se sobrou algo e a geração ainda é válida
            if !batch.is_empty() && gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
                let _ = file_entry_sender.send((my_gen, batch));
                ctx.request_repaint();
            }

            // Envia vetor VAZIO para sinalizar FIM do carregamento (apenas se a geração for a mesma)
            if gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
                let scan_elapsed = scan_start.elapsed();
                eprintln!("[PERF] Folder scan complete: {:?} took {:.2}s", current_path, scan_elapsed.as_secs_f64());
                let _ = file_entry_sender.send((my_gen, Vec::new()));
                ctx.request_repaint();
            }
        });
    }

    /// Navega para um caminho, adicionando ao histÃ³rico (corta histÃ³rico futuro)
    fn navigate_to(&mut self, path: &str) {
        // Normaliza paths de drive roots: garante que "Z:" sempre vire "Z:\"
        // Isso corrige o bug do PathBuf::join não adicionar backslash
        let normalized_path = if path.len() >= 2 && path.chars().nth(1) == Some(':') {
            // É um path Windows com letra de drive
            if path.len() == 2 {
                // Apenas "Z:" -> "Z:\"
                format!("{}\\", path)
            } else if path.chars().nth(2) != Some('\\') {
                // "Z:folder" -> "Z:\folder" (corrige path malformado)
                format!("{}\\{}", &path[0..2], &path[2..])
            } else {
                path.to_string()
            }
        } else {
            path.to_string()
        };

        // Se jÃ¡ estamos nesse caminho, nÃ£o faz nada
        if self.current_path == normalized_path {
            return;
        }

        // Corta histÃ³rico "futuro" (se voltamos e navegamos para outro lugar)
        if self.history_index < self.navigation_history.len().saturating_sub(1) {
            self.navigation_history.truncate(self.history_index + 1);
        }

        // Adiciona novo caminho ao histÃ³rico
        self.navigation_history.push(normalized_path.clone());
        self.history_index = self.navigation_history.len() - 1;

        self.current_path = normalized_path.clone();
        self.path_input = normalized_path.clone();
        self.is_computer_view = false;
        self.is_recycle_bin_view = false;  // Reset quando navega para qualquer pasta

        // SYNC TAB STATE
        self.sync_to_tab();

        self.reset_selection_and_search();

        // ATUALIZA O VIGIA
        self.watch_current_folder();

        self.load_folder(false);
    }

    /// Volta no histórico (sem adicionar ao histórico)
    fn go_back(&mut self) {
        if self.can_go_back() {
            // Guarda o path atual antes de voltar (para invalidar o preview)
            let previous_path = std::path::PathBuf::from(&self.current_path);
            
            self.history_index -= 1;
            let path = self.navigation_history[self.history_index].clone();

            if path == "Este Computador" {
                // Invalida preview da pasta que estávamos
                self.cache_manager.invalidate_folder_preview(&previous_path);
                
                // SYNC TAB STATE
                self.sync_to_tab();
                
                self.reset_selection_and_search();
                self.setup_computer_view();
            } else if path == "Lixeira" {
                // Invalida preview da pasta que estávamos
                self.cache_manager.invalidate_folder_preview(&previous_path);
                
                self.reset_selection_and_search();
                self.setup_recycle_bin_view();
            } else {
                let new_path = std::path::PathBuf::from(&path);
                
                // Se estávamos em uma subpasta do destino, invalida o preview dessa subpasta
                if previous_path.starts_with(&new_path) && previous_path != new_path {
                    self.cache_manager.invalidate_folder_preview(&previous_path);
                }
                
                self.current_path = path.clone();
                self.sync_to_tab();
                self.path_input = self.current_path.clone();
                self.is_computer_view = false;
                self.is_recycle_bin_view = false;
                self.reset_selection_and_search();
                self.watch_current_folder(); // Atualiza o watcher
                self.load_folder(false);
            }
        }
    }

    /// Avança no histórico
    fn go_forward(&mut self) {
        if self.history_index + 1 < self.navigation_history.len() {
            // Guarda o path atual antes de avançar (para invalidar o preview)
            let previous_path = std::path::PathBuf::from(&self.current_path);
            
            self.history_index += 1;
            let path = self.navigation_history[self.history_index].clone();

            if path == "Este Computador" {
                self.cache_manager.invalidate_folder_preview(&previous_path);
                
                // SYNC TAB STATE
                self.sync_to_tab();
                
                self.reset_selection_and_search();
                self.setup_computer_view();
            } else if path == "Lixeira" {
                self.cache_manager.invalidate_folder_preview(&previous_path);
                
                self.reset_selection_and_search();
                self.setup_recycle_bin_view();
            } else {
                let new_path = std::path::PathBuf::from(&path);
                
                // Se o destino é pai do path atual, invalida o preview do path atual
                if previous_path.starts_with(&new_path) && previous_path != new_path {
                    self.cache_manager.invalidate_folder_preview(&previous_path);
                }
                
                self.current_path = path.clone();
                self.sync_to_tab();
                self.path_input = self.current_path.clone();
                self.is_computer_view = false;
                self.is_recycle_bin_view = false;
                self.reset_selection_and_search();
                self.watch_current_folder(); // Atualiza o watcher
                self.load_folder(false);
            }
        }
    }

    /// Navega para "Este Computador" view (adicionando ao histórico)
    fn navigate_to_computer(&mut self) {
        if self.is_computer_view {
            return;
        }

        self.reset_selection_and_search();

        // Corta histórico "futuro"
        if self.history_index < self.navigation_history.len().saturating_sub(1) {
            self.navigation_history.truncate(self.history_index + 1);
        }

        // Adiciona ao histórico
        self.navigation_history.push("Este Computador".to_string());
        self.history_index = self.navigation_history.len() - 1;

        // SYNC TAB STATE
        self.tab_manager.active_mut().navigate_to("Este Computador");

        let _ = self.reload_drive_list();
        self.last_drive_refresh = Instant::now();
        self.setup_computer_view();
    }

    /// Navega para a Lixeira (adicionando ao histórico)
    fn navigate_to_recycle_bin(&mut self) {
        if self.is_recycle_bin_view {
            return;
        }

        self.reset_selection_and_search();

        // Corta histórico "futuro"
        if self.history_index < self.navigation_history.len().saturating_sub(1) {
            self.navigation_history.truncate(self.history_index + 1);
        }

        // Adiciona ao histórico
        self.navigation_history.push("Lixeira".to_string());
        self.history_index = self.navigation_history.len() - 1;

        // SYNC TAB STATE
        self.tab_manager.active_mut().navigate_to("Lixeira");

        self.setup_recycle_bin_view();
    }

    /// Configura a visão da Lixeira de forma ASSÍNCRONA
    fn setup_recycle_bin_view(&mut self) {
        self.current_path = "Lixeira".to_string();
        self.is_computer_view = false;
        self.is_recycle_bin_view = true;
        self.path_input = "Lixeira".to_string();
        self.is_loading_folder = true;
        self.items = Arc::new(Vec::new());
        self.all_items.clear();
        self.total_items = 0;

        // Incrementa geração para invalidar thumbnails antigos
        self.generation += 1;
        self.current_generation
            .store(self.generation, AtomicOrdering::Relaxed);

        let my_gen = self.generation;
        let gen_clone = self.current_generation.clone();
        let file_entry_sender = self.file_entry_sender.clone();
        let ctx = self.ui_ctx.clone();

        // Carrega itens da lixeira em thread separada (ASYNC) com batching
        std::thread::spawn(move || {
            use mtt_file_manager::infrastructure::windows::recycle_bin::enumerate_recycle_bin;

            // Enumera itens da lixeira via COM
            match enumerate_recycle_bin() {
                Ok(recycle_items) => {
                    const BATCH_SIZE: usize = 100;
                    let mut batch = Vec::with_capacity(BATCH_SIZE);
                    
                    for item in recycle_items {
                        // Verifica se a geração ainda é válida (cancelamento rápido)
                        if gen_clone.load(AtomicOrdering::Relaxed) != my_gen {
                            return;
                        }

                        // Cria um path "virtual" baseado na extensão para carregar ícone correto
                        // O path real não existe mais, mas o ícone é baseado na extensão
                        // O path real ($R) é necessário para ler a data de exclusão ($I creation time)
                        // Se physical_path estiver vazio (falha ao ler), usamos a lógica antiga de dummy.
                        let file_path = if !item.physical_path.as_os_str().is_empty() {
                            item.physical_path.clone()
                        } else if item.is_directory {
                             PathBuf::from("C:\\folder")
                        } else if !item.extension.is_empty() {
                             PathBuf::from(format!("dummy{}", item.extension))
                        } else {
                             item.original_path.clone()
                        };

                        let entry = FileEntry {
                            path: file_path, // Path físico ($R) para permitir get_deletion_date
                            name: item.name,
                            is_dir: item.is_directory,
                            size: item.size,
                            modified: 0,
                            folder_cover: None,
                            drive_info: None,
                            sync_status: mtt_file_manager::domain::file_entry::SyncStatus::None,
                            deletion_date: Some(item.date_deleted),
                        };
                        batch.push(entry);

                        // Envia batch quando cheio
                        if batch.len() >= BATCH_SIZE {
                            if gen_clone.load(AtomicOrdering::Relaxed) != my_gen {
                                return;
                            }
                            let _ = file_entry_sender.send((my_gen, std::mem::take(&mut batch)));
                            ctx.request_repaint();
                            batch = Vec::with_capacity(BATCH_SIZE);
                        }
                    }

                    // Envia itens restantes
                    if !batch.is_empty() && gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
                        let _ = file_entry_sender.send((my_gen, batch));
                        ctx.request_repaint();
                    }

                    // Sinal de fim do carregamento
                    if gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
                        let _ = file_entry_sender.send((my_gen, Vec::new()));
                        ctx.request_repaint();
                    }
                }
                Err(e) => {
                    eprintln!("[RECYCLE BIN] Erro ao enumerar: {:?}", e);
                    let _ = file_entry_sender.send((my_gen, Vec::new()));
                    ctx.request_repaint();
                }
            }
        });
    }

    /// Configura a visão de "Este Computador" sem afetar o histórico
    fn setup_computer_view(&mut self) {
        // Set computer view
        self.current_path = "Este Computador".to_string();
        self.is_computer_view = true;
        self.is_recycle_bin_view = false;
        self.path_input = "Este Computador".to_string();

        // ALWAYS reload drives to ensure fresh data
        let _ = self.reload_drive_list();

        // Populate items with drives
        use mtt_file_manager::domain::file_entry::DriveInfo;
        use mtt_file_manager::infrastructure::windows::get_volume_info;

        let mut computer_items = Vec::new();
        for (path, label) in &self.disks {
            let vol = get_volume_info(path);
            let drive_type = windows_infra::detect_drive_type(path);
            let mut entry = FileEntry::from_path(PathBuf::from(path), true);
            entry.name = label.clone();
            entry.drive_info = Some(DriveInfo {
                file_system: vol.file_system,
                total_space: vol.total_space,
                free_space: vol.free_space,
                drive_type,
            });
            computer_items.push(entry);
        }

        self.all_items = computer_items.clone();
        self.items = Arc::new(computer_items);
        self.reset_selection_and_search();
        self.total_items = self.disks.len();
        self.is_loading_folder = false; // CRITICAL: Clear loading state for computer view
    }

    fn reload_drive_list(&mut self) -> bool {
        let new_disks = get_all_drives();
        if new_disks != self.disks {
            self.disks = new_disks;
            true
        } else {
            false
        }
    }

    fn refresh_drives_if_needed(&mut self) {
        if self.last_drive_refresh.elapsed() >= Duration::from_millis(DRIVE_REFRESH_INTERVAL_MS) {
            self.last_drive_refresh = Instant::now();
            if self.reload_drive_list() && self.is_computer_view {
                self.setup_computer_view();
            }
        }
    }

    fn trigger_manual_refresh(&mut self) {
        if self.is_computer_view {
            let _ = self.reload_drive_list();
            self.setup_computer_view();
            self.last_drive_refresh = Instant::now();
        } else {
            self.load_folder(true);
        }
    }

    /// Sincroniza o estado atual do app para a aba ativa
    fn sync_to_tab(&mut self) {
        let active = self.tab_manager.active_mut();
        active.path = self.current_path.clone();
        active.path_input = self.path_input.clone();
        active.is_computer_view = self.is_computer_view;
        active.navigation_history = self.navigation_history.clone();
        active.history_index = self.history_index;
        active.items = self.items.clone();
        active.all_items = self.all_items.clone();
        active.selected_item = self.selected_item;
        active.selected_file = self.selected_file.clone();
        active.search_query = self.search_query.clone();
        active.scroll_to_selected = self.scroll_to_selected;

        // No Windows, Path::new("Este Computador").file_name() Ã© None
        if active.is_computer_view {
            active.title = "Este Computador".to_string();
        } else {
            active.title = Path::new(&active.path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| active.path.clone());
        }
    }

    /// Sincroniza o estado da aba ativa para o app
    fn sync_from_tab(&mut self) {
        // Clonamos o estado da aba para evitar problemas de borrow checker ao atualizar self
        let active = self.tab_manager.active().clone();
        self.current_path = active.path;
        self.path_input = active.path_input;
        self.is_computer_view = active.is_computer_view;
        self.navigation_history = active.navigation_history;
        self.history_index = active.history_index;
        self.items = active.items;
        self.all_items = active.all_items;
        self.selected_item = active.selected_item;
        self.selected_file = active.selected_file;
        self.search_query = active.search_query;
        self.scroll_to_selected = active.scroll_to_selected;

        self.watch_current_folder();
    }

    /// Sobe um nível (adiciona ao histórico)
    fn go_up_one_level(&mut self) {
        if let Some(parent) = Path::new(&self.current_path).parent() {
            let parent_str = parent.to_string_lossy().to_string();
            // No Windows, parent de "C:\" é vazio ou "." dependendo de como foi criado
            if !parent_str.is_empty() && parent_str != "." && parent_str != self.current_path {
                self.navigate_to(&parent_str);
                return;
            }
        }

        // Se já estamos no root de um drive ou local inválido, vai para Computador
        if !self.is_computer_view {
            self.navigate_to_computer();
        }
    }

    /// Configura o monitoramento da pasta atual
    fn watch_current_folder(&mut self) {
        let current_path = self.current_path.clone();

        // Canonicaliza o path para compatibilidade com Windows
        let path_to_watch = if let Ok(p) = Path::new(&current_path).canonicalize() {
            p
        } else {
            PathBuf::from(&current_path)
        };

        // Se o watcher já existe, apenas troca o path monitorado
        if let Some(ref mut _watcher) = self.watcher {
            // Para de monitorar todos os paths antigos (o watcher pode ter múltiplos)
            // Como não temos referência ao path antigo, vamos recriar o watcher
            // (notify não tem API para listar paths monitorados)
        }

        // Cria ou recria o watcher
        let tx = self.fs_event_sender.clone();
        let ctx_clone = self.ui_ctx.clone();

        let watcher_result =
            notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                let _ = tx.send(res);
                ctx_clone.request_repaint();
            });

        if let Ok(mut watcher) = watcher_result {
            if let Err(_e) = watcher.watch(&path_to_watch, RecursiveMode::NonRecursive) {
                // Silently fail - watcher is optional
            } else {
                self.watcher = Some(watcher);
            }
        }
    }

    /// Renomeia arquivo usando Shell API (suporta Undo/Ctrl+Z)
    fn rename_with_shell(&mut self, idx: usize) {
        if let Some((_, new_name)) = self.renaming_state.take() {
            if let Some(item) = self.items.get(idx) {
                let old_path = item.path.to_string_lossy().to_string();
                if let Some(parent) = item.path.parent() {
                    let new_path = parent.join(&new_name).to_string_lossy().to_string();

                    // Regra da API: Strings devem terminar com DOIS nulls (\0\0)
                    let mut from_vec: Vec<u16> = old_path.encode_utf16().collect();
                    from_vec.push(0);
                    from_vec.push(0);

                    let mut to_vec: Vec<u16> = new_path.encode_utf16().collect();
                    to_vec.push(0);
                    to_vec.push(0);

                    let mut op = SHFILEOPSTRUCTW {
                        hwnd: HWND(std::ptr::null_mut()),
                        wFunc: FO_RENAME,
                        pFrom: PCWSTR(from_vec.as_ptr()),
                        pTo: PCWSTR(to_vec.as_ptr()),
                        fFlags: FOF_ALLOWUNDO.0 as u16,
                        ..Default::default()
                    };

                    unsafe {
                        // SAFETY: `from_vec` and `to_vec` are properly double-null terminated wide strings
                        // as required by `SHFileOperationW`.
                        let result = SHFileOperationW(&mut op);
                        if result == 0 {
                            // Sucesso: Recarrega a pasta para atualizar a UI
                            self.load_folder(false);
                        } else {
                            eprintln!("Erro ao renomear via Shell: {}", result);
                        }
                    }
                }
            }
        }
    }

    /// Pode voltar no histÃ³rico?
    fn can_go_back(&self) -> bool {
        self.history_index > 0
    }

    /// Pode avanÃ§ar no histÃ³rico?
    fn can_go_forward(&self) -> bool {
        self.history_index < self.navigation_history.len().saturating_sub(1)
    }

    fn request_thumbnail_load(&self, path: PathBuf) {
        // Envia pedido para o Worker Pool com a geraÃ§Ã£o atual
        let _ = self.thumbnail_req_sender.send((path, self.generation));
    }

    fn request_folder_preview_load(&mut self, path: PathBuf) {
        if self.cache_manager.start_folder_preview_loading(path.clone()) {
            let _ = self.folder_preview_sender.send(path);
        }
    }

    /// Captura e armazena o HWND nativo a partir do título da janela principal.
    fn ensure_window_handle(&mut self, _frame: &eframe::Frame) {
        if self.native_hwnd.is_some() {
            return;
        }

        let title: Vec<u16> = "MTT File Manager"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let hwnd_result = unsafe { FindWindowW(None, PCWSTR(title.as_ptr())) };
        if let Ok(hwnd) = hwnd_result {
            if !hwnd.0.is_null() {
                self.native_hwnd = Some(hwnd);
                
                // Apply rounded corners (Windows 11 style)
                unsafe {
                    let corner_pref = DWMWCP_ROUND;
                    let result = DwmSetWindowAttribute(
                        hwnd,
                        DWMWA_WINDOW_CORNER_PREFERENCE,
                        &corner_pref as *const _ as *const _,
                        std::mem::size_of::<u32>() as u32,
                    );
                    if result.is_ok() {
                        eprintln!("[DWM] Rounded corners applied successfully");
                    } else {
                        eprintln!("[DWM] Failed to apply rounded corners: {:?}", result);
                    }
                }
                
                // Pre-initialize shell extensions so they're ready on first context menu
                mtt_file_manager::infrastructure::windows::native_menu::warmup_shell_extensions(hwnd);
            }
        }
    }


    /// Retorna icone para um arquivo, carregando sob demanda.
    /// Executaveis (.exe, .lnk, .ico) sao cacheados por path completo.
    /// Demais extensoes sao cacheadas por tipo.
    fn get_or_load_icon(
        &mut self,
        ctx: &egui::Context,
        path: &Path,
    ) -> Option<egui::TextureHandle> {
        let extension = path.extension()?.to_str()?.to_lowercase();

        // Decide cache key: path completo para executaveis, extensao para demais
        let cache_key = if matches!(extension.as_str(), "exe" | "lnk" | "ico") {
            // Cache por path completo - cada executavel tem icone unico
            path.to_string_lossy().to_string()
        } else {
            // Cache por extensao - todos .txt compartilham icone
            format!(".{}", extension)
        };

        // Cache hit? Clone do handle (barato)
        if let Some(texture) = self.cache_manager.icon_cache.get(&cache_key) {
            return Some(texture.clone());
        }

        // Cache miss -> carrega icone
        let thumbnail_size = self.thumbnail_size;
        let icon_size = if thumbnail_size < 100.0 {
            IconSize::Small
        } else {
            IconSize::Large
        };

        // Para executaveis, usa path real; para demais, usa extensao dummy com USEFILEATTRIBUTES
        let icon_result = if matches!(extension.as_str(), "exe" | "lnk" | "ico") {
            extract_file_icon_by_path(path, icon_size)
        } else {
            mtt_file_manager::infrastructure::windows::get_file_type_icon(false, &format!(".{}", extension), icon_size)
        };

        match icon_result {
            Ok((rgba_data, width, height)) => {
                let texture = ctx.load_texture(
                    format!("icon_{}", cache_key),
                    egui::ColorImage::from_rgba_unmultiplied(
                        [width as usize, height as usize],
                        &rgba_data,
                    ),
                    egui::TextureOptions::NEAREST,
                );

                let cloned = texture.clone();
                self.cache_manager.icon_cache.put(cache_key, texture);
                Some(cloned)
            }
            Err(_) => None, // Fallback: sem icone
        }
    }

    /// Garante que Ã­cone de pasta estÃ¡ carregado.
    fn ensure_folder_icon(&mut self, ctx: &egui::Context) {
        let thumbnail_size = self.thumbnail_size;
        let icon_size = if thumbnail_size < 100.0 {
            IconSize::Small
        } else {
            IconSize::Large
        };

        self.cache_manager
            .ensure_folder_icon(ctx, || windows_infra::extract_folder_icon(icon_size));
    }

    /// Garante que Ã­cone de "Este Computador" estÃ¡ carregado.
    fn ensure_computer_icon(&mut self, ctx: &egui::Context) {
        self.cache_manager.ensure_computer_icon(ctx, || {
            windows_infra::extract_computer_icon(IconSize::Small)
        });
    }

    /// Lazily refresh media metadata for the currently selected file.
    fn refresh_selected_metadata(&mut self) {
        let current_file = self
            .selected_file
            .as_ref()
            .filter(|f| !f.is_dir)
            .map(|f| f.path.clone());

        match current_file {
            Some(path) => {
                let mtime = std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);

                if let Some((cached_mtime, meta)) = self.metadata_cache.get(&path) {
                    if *cached_mtime == mtime {
                        self.selected_metadata = Some((path, meta.clone()));
                        return;
                    }
                }

                if !self.metadata_loading.contains(&path) {
                    let _ = self.metadata_req_sender.send((path.clone(), mtime));
                    self.metadata_loading.insert(path.clone());
                }

                if !matches!(self.selected_metadata.as_ref(), Some((p, _)) if p == &path) {
                    self.selected_metadata = None;
                }
            }
            None => {
                self.selected_metadata = None;
            }
        }
    }

    fn format_media_duration(ticks_100ns: u64) -> String {
        // 1 tick = 100ns; 10_000_000 ticks = 1s
        let total_seconds = ticks_100ns / 10_000_000;
        let hours = total_seconds / 3600;
        let minutes = (total_seconds % 3600) / 60;
        let seconds = total_seconds % 60;

        if hours > 0 {
            format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
        } else {
            format!("{:02}:{:02}", minutes, seconds)
        }
    }

    fn format_bitrate(bps: u32) -> String {
        let bps = bps as f64;
        if bps >= 1_000_000.0 {
            format!("{:.1} Mbps", bps / 1_000_000.0)
        } else if bps >= 1_000.0 {
            format!("{:.0} Kbps", bps / 1_000.0)
        } else {
            format!("{:.0} bps", bps)
        }
    }

    fn approximate_bitrate(size_bytes: u64, duration_100ns: u64) -> Option<u32> {
        if duration_100ns == 0 {
            return None;
        }
        let seconds = duration_100ns as f64 / 10_000_000.0;
        if seconds <= 0.0 {
            return None;
        }
        let bits_per_sec = (size_bytes as f64 * 8.0) / seconds;
        Some(bits_per_sec.max(0.0) as u32)
    }

    /// Processa mensagens que chegam dos canais de workers
    fn process_incoming_messages(&mut self, ctx: &egui::Context) {
        // 1. CHECK DE REFRESH MANUAL (F5)
        if ctx.input(|i| i.key_pressed(egui::Key::F5)) {
            self.trigger_manual_refresh();
        }

        while self.device_event_receiver.try_recv().is_ok() {
            if self.reload_drive_list() {
                self.last_drive_refresh = Instant::now();
                if self.is_computer_view {
                    self.setup_computer_view();
                }
                // Force immediate repaint without waiting for input events
                ctx.request_repaint_after(std::time::Duration::from_millis(0));
            }
        }

        // 2. CHECK DE AUTO-REFRESH (WATCHER)
        // 2. CHECK DE AUTO-REFRESH (WATCHER)
        fn normalize_for_match(p: &Path) -> String {
            let s = p.to_string_lossy().to_string().to_lowercase();
            if s.starts_with(r"\\?\") {
                s[4..].to_string()
            } else {
                s
            }
        }

        fn clean_path(p: &Path) -> PathBuf {
            let s = p.to_string_lossy().to_string();
            if s.starts_with(r"\\?\") {
                PathBuf::from(&s[4..])
            } else {
                p.to_path_buf()
            }
        }

        let current_path_norm = normalize_for_match(Path::new(&self.current_path));
        
        while let Ok(event) = self.fs_event_receiver.try_recv() {
            match event {
                Ok(evt) => {
                    // Detecta eventos de Remove para limpar cache automaticamente
                    if matches!(evt.kind, notify::EventKind::Remove(_)) {
                        for path in &evt.paths {
                            let cleaned = clean_path(path);
                            eprintln!("[FS] Detected removal, clearing disk cache for: {:?}", cleaned);
                            self.disk_cache.remove_cache_for_path(&cleaned);
                        }
                    }
                    
                    // Detecta Modify para invalidar folder previews
                    for path in &evt.paths {
                        // 1. Se o path alterado é uma subpasta direta da pasta atual
                        if let Some(parent) = path.parent() {
                            let parent_norm = normalize_for_match(parent);
                            if parent_norm == current_path_norm {
                                let cleaned = clean_path(path);
                                eprintln!("[FS] Direct subfolder modified: {:?}", cleaned.file_name());
                                self.cache_manager.invalidate_folder_preview(&cleaned);
                            }
                        }
                        
                        // 2. Se o path alterado é UM ARQUIVO dentro de uma subpasta da pasta atual
                        if let Some(parent) = path.parent() {
                            if let Some(grandparent) = parent.parent() {
                                let grandparent_norm = normalize_for_match(grandparent);
                                if grandparent_norm == current_path_norm {
                                    let cleaned_parent = clean_path(parent);
                                    eprintln!("[FS] File in subfolder modified, invalidating: {:?}", cleaned_parent.file_name());
                                    self.cache_manager.invalidate_folder_preview(&cleaned_parent);
                                }
                            }
                        }
                    }
                    
                    self.pending_auto_reload = true;
                }
                Err(e) => eprintln!("Erro de watch: {:?}", e),
            }
        }

        // Executa reload apenas quando debounce permitir
        if self.pending_auto_reload {
            let elapsed = self.last_auto_reload.elapsed();
            if elapsed > Duration::from_millis(500) {
                // VALIDA SE O PATH ATUAL AINDA EXISTE (pode ter sido renomeado/deletado)
                if Path::new(&self.current_path).exists() {
                    self.load_folder(true); // force_refresh para atualizar thumbnails modificados
                } else {
                    self.go_up_one_level();
                }
                self.last_auto_reload = Instant::now();
                self.pending_auto_reload = false;
            }
        }

        // 1. STREAMING: Recebe lotes incrementais de FileEntry (Filtrado por geraÃ§Ã£o)
        while let Ok((gen_id, new_batch)) = self.file_entry_receiver.try_recv() {
            if gen_id != self.generation {
                continue; // Descarta dados de uma navegaÃ§Ã£o/refresh anterior
            }

            if new_batch.is_empty() {
                // Lote vazio = Sinal de "Fim do Carregamento" da thread
                self.is_loading_folder = false;
                // OrdenaÃ§Ã£o final para garantir tudo correto
                self.sort_items();
            } else {
                // Chegou dados! Adiciona Ã  lista mestre
                self.all_items.extend(new_batch);

                // Reaplica filtro (que já chama sort_items internamente)
                self.filter_items();
            }
            ctx.request_repaint();
        }

        // 2. Cover Worker: Recebe resultados de capas de folder
        let mut folder_updates = false;
        while let Ok((folder_path, cover_opt)) = self.cover_worker_receiver.try_recv() {
            if let Some(cover) = cover_opt {
                // Atualiza em all_items (fonte mutável)
                if let Some(item) = self.all_items.iter_mut().find(|i| i.path == folder_path) {
                    item.folder_cover = Some(cover.clone());
                    self.disk_cache.set_folder_cover(&folder_path, &cover);
                    folder_updates = true;

                    // Requisita thumbnail se necessário (Marcando como em carregamento para evitar loop)
                    if !self.cache_manager.has_thumbnail(&cover)
                        && self.cache_manager.start_loading(cover.clone())
                    {
                        self.request_thumbnail_load(cover);
                    }
                }
            }
        }
        // Reconstrói items a partir de all_items se houve updates
        if folder_updates {
            self.filter_items();
            ctx.request_repaint();
        }

        // 3. Icon Worker: Recebe resultados de ícones assíncronos
        while let Ok((path, pixels, width, height)) = self.icon_res_receiver.try_recv() {
            self.loading_icons.remove(&path);

            // Carrega textura no cache de ícones
            let cache_key = path.to_string_lossy().to_string();
            if !self.item_icon_loader.icon_cache.contains(&cache_key) {
                let texture = ctx.load_texture(
                    cache_key.clone(),
                    egui::ColorImage::from_rgba_unmultiplied(
                        [width as usize, height as usize],
                        &pixels,
                    ),
                    egui::TextureOptions::NEAREST,
                );
                self.item_icon_loader.icon_cache.put(cache_key, texture);
            }
        }

        // 4. Metadata Worker: drena respostas mesmo sem thumbnails
        let mut metadata_updated = false;
        while let Ok((path, mtime, meta)) = self.metadata_res_receiver.try_recv() {
            self.metadata_loading.remove(&path);
            self.metadata_cache.put(path.clone(), (mtime, meta.clone()));

            if let Some(selected) = &self.selected_file {
                if selected.path == path {
                    self.selected_metadata = Some((path.clone(), meta));
                    metadata_updated = true;
                }
            }
        }
        if metadata_updated {
            ctx.request_repaint();
        }

        // 5. Individual thumbnails
        let mut received_any = false;
        let mut _new_items_added = false;

        while let Ok(thumbnail_data) = self.image_receiver.try_recv() {
            // --- VALIDAÇÃO DE MEMÓRIA ---
            // Se a imagem pertence a uma geração anterior (outra folder), descarta.
            if thumbnail_data.generation != self.generation {
                continue;
            }
            // ----------------------------

            received_any = true;

            // SÃ³ processa thumbnails (image_data nÃ£o vazio)
            if !thumbnail_data.image_data.is_empty() {
                self.cache_manager.finish_loading(&thumbnail_data.path);

                let texture = ctx.load_texture(
                    thumbnail_data.path.to_string_lossy().to_string(),
                    egui::ColorImage::from_rgba_unmultiplied(
                        [
                            thumbnail_data.width as usize,
                            thumbnail_data.height as usize,
                        ],
                        &thumbnail_data.image_data,
                    ),
                    egui::TextureOptions::NEAREST,
                );

                self.cache_manager
                    .put_thumbnail(thumbnail_data.path.clone(), texture.clone());

                // Update selected_thumbnail if it matches the selected_file
                if let Some(selected_file) = &self.selected_file {
                    if selected_file.path == thumbnail_data.path {
                        self.selected_thumbnail = Some(texture);
                    }
                }
            }
        }

        // 6. Folder Previews (Native Sandwich effect)
        while let Ok(data) = self.folder_preview_receiver.try_recv() {
            self.cache_manager.finish_folder_preview_loading(&data.path);

            let texture = ctx.load_texture(
                format!("folder_preview_{}", data.path.to_string_lossy()),
                egui::ColorImage::from_rgba_unmultiplied(
                    [data.width as usize, data.height as usize],
                    &data.rgba_data,
                ),
                egui::TextureOptions::NEAREST,
            );

            self.cache_manager.put_folder_preview(data.path, texture);
        }

        // 9. FOLDER SIZE RESULTS
        while let Ok((folder_path, total_size)) = self.folder_size_res_receiver.try_recv() {
            self.folder_size_loading.remove(&folder_path);
            self.folder_size_cache.insert(folder_path, total_size);
            received_any = true;
        }

        if received_any {
            ctx.request_repaint();
        }
    }

    // --- DETALHES (LIST VIEW) ---
    fn render_list_view(&mut self, ui: &mut egui::Ui) {
        use mtt_file_manager::ui::views::{list_view, ListViewContext, ListViewOperations};

        // Keyboard navigation for list view (ONLY when not renaming)
        // Throttle: 50ms between navigations to prevent scroll desync when holding keys
        if self.renaming_state.is_none() && self.last_keyboard_nav.elapsed() >= Duration::from_millis(50) {
            let current_index = self.items.iter().position(|x| {
                self.selected_file
                    .as_ref()
                    .map_or(false, |f| f.path == x.path)
            });

            let mut new_index = None;
            if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
                new_index = current_index.map(|idx| idx + 1).or(Some(0));
            } else if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
                new_index = current_index.map(|idx| idx.saturating_sub(1));
            }

            if let Some(idx) = new_index {
                let clamped = idx.min(self.items.len().saturating_sub(1));
                if let Some(item) = self.items.get(clamped) {
                    let item_path = item.path.clone();
                    let is_dir = item.is_dir;
                    
                    self.selected_file = Some(item.clone());
                    self.selected_item = Some(clamped);
                    self.update_selected_thumbnail();
                    self.scroll_to_selected = true; // Trigger scroll to selected item
                    self.last_keyboard_nav = Instant::now(); // Reset throttle timer
                    
                    // Trigger thumbnail load for sidebar preview
                    if !is_dir {
                        if !self.cache_manager.has_thumbnail(&item_path)
                            && !self.cache_manager.is_loading(&item_path)
                        {
                            self.request_thumbnail_load(item_path);
                        }
                    }
                }
            }

            // Enter to open (only when not renaming)
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                if let Some(selected) = &self.selected_file.clone() {
                    if selected.is_dir {
                        self.navigate_to(&selected.path.to_string_lossy());
                        return; // Exit early after navigation
                    } else {
                        open_with_shell(&selected.path);
                    }
                }
            }
        }

        // Extrair dados necessários para evitar múltiplos borrows
        let items = self.items.clone(); // Clone para evitar borrow
        let selected_item = self.selected_item;
        let selected_file = self.selected_file.clone();
        let sort_mode = self.sort_mode;
        let sort_descending = self.sort_descending;
        let renaming_state = self.renaming_state.clone();
        let focus_rename = self.focus_rename;
        let folder_icon_texture = self.cache_manager.folder_icon_texture.clone();
        let computer_icon = self.cache_manager.computer_icon.clone();

        // Check if current path is in OneDrive
        let is_onedrive_folder = mtt_file_manager::infrastructure::onedrive::is_onedrive_path(
            &PathBuf::from(&self.current_path),
        );

        // Criar contexto com referências mutáveis separadas
        let scroll_to_selected = self.scroll_to_selected;
        let mut ctx = ListViewContext {
            items: &items,
            selected_item,
            selected_file: selected_file.as_ref(),
            sort_mode,
            sort_descending,
            renaming_state: renaming_state.clone(),
            focus_rename,
            scroll_to_selected,
            is_computer_view: self.is_computer_view,
            is_recycle_bin_view: self.is_recycle_bin_view,
            is_onedrive_folder,
            texture_cache: &mut self.cache_manager.texture_cache,
            loading_set: &mut self.cache_manager.loading_set,
            scanned_folders: &mut self.scanned_folders,
            folder_icon_texture: folder_icon_texture.as_ref(),
            computer_icon: computer_icon.as_ref(),
            drive_icon_cache: &mut self.cache_manager.drive_icon_cache,
            item_icon_loader: &mut self.item_icon_loader,
            deletion_date_cache: Some(&mut self.deletion_date_cache),
        };

        // Usar uma abordagem diferente: coletar ações em vetores
        let mut actions = Vec::new();

        struct ListOps<'a> {
            actions: &'a mut Vec<ListAction>,
        }

        enum ListAction {
            NavigateTo(String),
            OpenWithShell(PathBuf),
            RequestThumbnailLoad(PathBuf),
            RequestFolderScan(PathBuf),
            RequestFolderPreviewLoad(PathBuf),
            RenameWithShell(usize),
        }

        impl ListViewOperations for ListOps<'_> {
            fn navigate_to(&mut self, path: &str) {
                self.actions.push(ListAction::NavigateTo(path.to_string()));
            }

            fn open_with_shell(&mut self, path: &PathBuf) {
                self.actions.push(ListAction::OpenWithShell(path.clone()));
            }

            fn request_thumbnail_load(&mut self, path: PathBuf) {
                self.actions.push(ListAction::RequestThumbnailLoad(path));
            }

            fn request_folder_scan(&mut self, path: PathBuf) {
                self.actions.push(ListAction::RequestFolderScan(path));
            }

            fn request_folder_preview_load(&mut self, path: PathBuf) {
                self.actions.push(ListAction::RequestFolderPreviewLoad(path));
            }

            fn rename_with_shell(&mut self, idx: usize) {
                self.actions.push(ListAction::RenameWithShell(idx));
            }
        }

        let mut ops = ListOps {
            actions: &mut actions,
        };

        let action = list_view::render_list_view(ui, &mut ctx, &mut ops);

        // Update state from context
        self.sort_mode = ctx.sort_mode;
        self.sort_descending = ctx.sort_descending;
        self.renaming_state = ctx.renaming_state;
        self.focus_rename = ctx.focus_rename;
        self.scroll_to_selected = false; // Reset after scrolling

        // Processar ações (bloqueadas durante renomeação)
        let is_renaming = self.renaming_state.is_some();
        match action {
            Some(list_view::ListViewAction::Click(idx)) if !is_renaming => {
                self.selected_item = Some(idx);
                if let Some(item) = self.items.get(idx) {
                    let item_path = item.path.clone();
                    let is_dir = item.is_dir;
                    
                    self.selected_file = Some(item.clone());
                    self.update_selected_thumbnail();

                    // Trigger thumbnail load for sidebar preview
                    if !is_dir {
                        if !self.cache_manager.has_thumbnail(&item_path)
                            && !self.cache_manager.is_loading(&item_path)
                        {
                            self.request_thumbnail_load(item_path);
                        }
                    }
                }
            }
            Some(list_view::ListViewAction::DoubleClick(idx)) if !is_renaming => {
                let path_to_navigate = self.items.get(idx).map(|item| {
                    if item.is_dir {
                        Some(item.path.clone())
                    } else {
                        open_with_shell(&item.path);
                        None
                    }
                });

                if let Some(Some(path)) = path_to_navigate {
                    self.navigate_to(&path.to_string_lossy());
                }
            }
            Some(list_view::ListViewAction::SecondaryClick(idx)) if !is_renaming => {
                // Step 1: Update selection immediately (this will cause a repaint)
                self.selected_item = Some(idx);
                if let Some(item) = self.items.get(idx) {
                    let item_path = item.path.clone();
                    self.selected_file = Some(item.clone());
                    self.context_menu.target_path = Some(item_path.clone());

                    // Usar o novo sistema de menu estilizado
                    let pointer_pos = ui.ctx().pointer_latest_pos().unwrap_or(egui::Pos2::ZERO);
                    self.populate_context_menu(ui.ctx(), &item_path, false, Some(idx));
                    self.context_menu.open(
                        pointer_pos,
                        Some(idx),
                        Some(item_path),
                        false,
                    );
                }
            }
            Some(list_view::ListViewAction::SortChange(mode)) => {
                // Toggle direction if same mode, otherwise switch mode
                if self.sort_mode == mode {
                    self.sort_descending = !self.sort_descending;
                } else {
                    self.sort_mode = mode;
                    self.sort_descending = false;
                }
                self.sort_items();
                self.save_preferences();
            }
            _ => {}
        }

        // Executar ações coletadas
        for action in actions {
            match action {
                ListAction::NavigateTo(path) => self.navigate_to(&path),
                ListAction::OpenWithShell(path) => open_with_shell(&path),
                ListAction::RequestThumbnailLoad(path) => self.request_thumbnail_load(path),
                ListAction::RequestFolderScan(path) => self.request_folder_scan(path),
                ListAction::RequestFolderPreviewLoad(path) => self.request_folder_preview_load(path),
                ListAction::RenameWithShell(idx) => self.rename_with_shell(idx),
            }
        }
    }

    // --- GRANDE (GRID VIEW) ---
    fn render_grid_view(&mut self, ui: &mut egui::Ui) {
        use mtt_file_manager::ui::views::{grid_view, GridViewContext, GridViewOperations};

        // Calculate cols for keyboard navigation
        let padding = 8.0;
        let item_w = self.thumbnail_size;
        let available_w = ui.available_width();
        let cols = ((available_w - padding) / (item_w + padding))
            .floor()
            .max(1.0) as usize;

        // Keyboard navigation (ONLY when not renaming)
        // Throttle: 50ms between navigations to prevent scroll desync when holding keys
        if self.renaming_state.is_none() && self.last_keyboard_nav.elapsed() >= Duration::from_millis(50) {
            let current_index = self.items.iter().position(|x| {
                self.selected_file
                    .as_ref()
                    .map_or(false, |f| f.path == x.path)
            });

            let mut new_index = None;
            if ui.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
                new_index = current_index.map(|idx| idx + 1).or(Some(0));
            } else if ui.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
                new_index = current_index.map(|idx| idx.saturating_sub(1));
            } else if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
                new_index = current_index.map(|idx| idx + cols).or(Some(0));
            } else if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
                new_index = current_index.map(|idx| idx.saturating_sub(cols));
            }

            if let Some(idx) = new_index {
                let clamped = idx.min(self.items.len().saturating_sub(1));
                if let Some(item) = self.items.get(clamped) {
                    self.selected_file = Some(item.clone());
                    self.selected_item = Some(clamped);
                    self.update_selected_thumbnail();
                    self.scroll_to_selected = true; // Trigger scroll to selected item
                    self.last_keyboard_nav = Instant::now(); // Reset throttle timer
                }
            }

            // Enter to open (only when not renaming)
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                if let Some(selected) = &self.selected_file.clone() {
                    if selected.is_dir {
                        self.navigate_to(&selected.path.to_string_lossy());
                        return; // Exit early after navigation
                    } else {
                        open_with_shell(&selected.path);
                    }
                }
            }
        }

        // Extrair dados necessários para evitar múltiplos borrows
        let items = self.items.clone(); // Clone para evitar borrow
        let selected_item = self.selected_item;
        let selected_file = self.selected_file.clone();
        let thumbnail_size = self.thumbnail_size;
        let last_grid_cols = self.last_grid_cols;
        let renaming_state = self.renaming_state.clone();
        let focus_rename = self.focus_rename;
        let folder_icon_texture = self.cache_manager.folder_icon_texture.clone();
        let computer_icon = self.cache_manager.computer_icon.clone();

        // Criar contexto com referências mutáveis separadas
        let scroll_to_selected = self.scroll_to_selected;
        let mut ctx = GridViewContext {
            items: &items,
            selected_item,
            selected_file: selected_file.as_ref(),
            thumbnail_size,
            last_grid_cols,
            renaming_state: renaming_state.clone(),
            focus_rename,
            scroll_to_selected,
            is_computer_view: self.is_computer_view,
            is_recycle_bin_view: self.is_recycle_bin_view,
            texture_cache: &mut self.cache_manager.texture_cache,
            loading_set: &mut self.cache_manager.loading_set,
            scanned_folders: &mut self.scanned_folders,
            folder_icon_texture: folder_icon_texture.as_ref(),
            computer_icon: computer_icon.as_ref(),
            drive_icon_cache: &mut self.cache_manager.drive_icon_cache,
            item_icon_loader: &mut self.item_icon_loader,
            folder_preview_cache: &mut self.cache_manager.folder_preview_cache,
            folder_preview_loading: &mut self.cache_manager.folder_preview_loading,
        };

        // Usar uma abordagem diferente: coletar ações em vetores
        let mut actions = Vec::new();

        struct GridOps<'a> {
            actions: &'a mut Vec<GridAction>,
        }

        enum GridAction {
            NavigateTo(String),
            OpenWithShell(PathBuf),
            RequestThumbnailLoad(PathBuf),
            RequestFolderScan(PathBuf),
            RequestFolderPreviewLoad(PathBuf),
            RenameWithShell(usize),
        }

        impl GridViewOperations for GridOps<'_> {
            fn navigate_to(&mut self, path: &str) {
                self.actions.push(GridAction::NavigateTo(path.to_string()));
            }

            fn open_with_shell(&mut self, path: &PathBuf) {
                self.actions.push(GridAction::OpenWithShell(path.clone()));
            }

            fn request_thumbnail_load(&mut self, path: PathBuf) {
                self.actions.push(GridAction::RequestThumbnailLoad(path));
            }

            fn request_folder_scan(&mut self, path: PathBuf) {
                self.actions.push(GridAction::RequestFolderScan(path));
            }
            fn request_folder_preview_load(&mut self, path: PathBuf) {
                self.actions.push(GridAction::RequestFolderPreviewLoad(path));
            }

            fn rename_with_shell(&mut self, idx: usize) {
                self.actions.push(GridAction::RenameWithShell(idx));
            }
        }

        let mut ops = GridOps {
            actions: &mut actions,
        };

        let action = grid_view::render_grid_view(ui, &mut ctx, &mut ops);

        // Update state from context
        self.last_grid_cols = ctx.last_grid_cols;
        self.renaming_state = ctx.renaming_state;
        self.focus_rename = ctx.focus_rename;
        self.scroll_to_selected = false; // Reset after scrolling

        // Processar ações (bloqueadas durante renomeação, exceto clique no próprio item)
        let is_renaming = self.renaming_state.is_some();
        match action {
            Some(grid_view::GridViewAction::Click(idx)) if !is_renaming => {
                self.selected_item = Some(idx);
                if let Some(item) = self.items.get(idx) {
                    self.selected_file = Some(item.clone());
                    self.update_selected_thumbnail();
                }
            }
            Some(grid_view::GridViewAction::DoubleClick(idx)) if !is_renaming => {
                let path_to_navigate = self.items.get(idx).map(|item| {
                    if item.is_dir {
                        Some(item.path.clone())
                    } else {
                        open_with_shell(&item.path);
                        None
                    }
                });

                if let Some(Some(path)) = path_to_navigate {
                    self.navigate_to(&path.to_string_lossy());
                }
            }
            Some(grid_view::GridViewAction::SecondaryClick(idx)) if !is_renaming => {
                // Step 1: Update selection immediately (this will cause a repaint)
                self.selected_item = Some(idx);
                if let Some(item) = self.items.get(idx) {
                    let item_path = item.path.clone();
                    self.selected_file = Some(item.clone());
                    self.context_menu.target_path = Some(item_path.clone());

                    // Usar o novo sistema de menu estilizado
                    let pointer_pos = ui.ctx().pointer_latest_pos().unwrap_or(egui::Pos2::ZERO);
                    self.populate_context_menu(ui.ctx(), &item_path, false, Some(idx));
                    self.context_menu.open(
                        pointer_pos,
                        Some(idx),
                        Some(item_path),
                        false,
                    );
                }
            }
            _ => {}
        }

        // Executar ações coletadas
        for action in actions {
            match action {
                GridAction::NavigateTo(path) => self.navigate_to(&path),
                GridAction::OpenWithShell(path) => open_with_shell(&path),
                GridAction::RequestThumbnailLoad(path) => self.request_thumbnail_load(path),
                GridAction::RequestFolderScan(path) => self.request_folder_scan(path),
                GridAction::RequestFolderPreviewLoad(path) => self.request_folder_preview_load(path),
                GridAction::RenameWithShell(idx) => self.rename_with_shell(idx),
            }
        }
    }

    fn render_item_slot(&mut self, ui: &mut egui::Ui, idx: usize) {
        if idx >= self.items.len() {
            return;
        }

        use mtt_file_manager::ui::components::item_slot::{render_item_slot, ItemSlotContext};

        // Clone item data to avoid borrowing self.items during the render
        let item = self.items[idx].clone();
        let is_renaming = self
            .renaming_state
            .as_ref()
            .map_or(false, |(i, _)| *i == idx);

        // Para evitar conflitos de borrow, coletamos as operações pendentes
        // e executamos depois de renderizar
        let mut pending_thumbnail_loads: Vec<std::path::PathBuf> = Vec::new();
        let mut pending_folder_scans: Vec<std::path::PathBuf> = Vec::new();
        let mut pending_folder_preview_loads: Vec<std::path::PathBuf> = Vec::new();
        let mut pending_rename: Option<usize> = None;

        // Texto de renomeação precisa ser tratado separadamente
        let mut renaming_text_clone = if is_renaming {
            self.renaming_state.as_ref().map(|(_, s)| s.clone())
        } else {
            None
        };

        // Create context with mutable reference to the clone
        {
            let renaming_text = renaming_text_clone.as_mut();

            let mut ctx = ItemSlotContext {
                item: &item,
                idx,
                thumbnail_size: self.thumbnail_size,
                is_renaming,
                renaming_text,
                focus_rename: self.focus_rename,
                is_recycle_bin_view: self.is_recycle_bin_view,
                texture_cache: &mut self.cache_manager.texture_cache,
                icon_loader: &mut self.item_icon_loader,
                scanned_folders: &mut self.scanned_folders,
                loading_set: &mut self.cache_manager.loading_set,
                folder_preview_cache: &mut self.cache_manager.folder_preview_cache,
                folder_preview_loading: &mut self.cache_manager.folder_preview_loading,
            };

            // Create simple ops struct that collects operations
            struct SimpleOps<'a> {
                thumbnail_loads: &'a mut Vec<std::path::PathBuf>,
                folder_scans: &'a mut Vec<std::path::PathBuf>,
                folder_preview_loads: &'a mut Vec<std::path::PathBuf>,
                pending_rename: &'a mut Option<usize>,
            }

            impl<'a> mtt_file_manager::ui::components::item_slot::ItemSlotOperations for SimpleOps<'a> {
                fn request_thumbnail_load(&mut self, path: std::path::PathBuf) {
                    self.thumbnail_loads.push(path);
                }

                fn request_folder_scan(&mut self, path: std::path::PathBuf) {
                    self.folder_scans.push(path);
                }

                fn request_folder_preview_load(&mut self, path: std::path::PathBuf) {
                    self.folder_preview_loads.push(path);
                }

                fn rename_item(&mut self, idx: usize) {
                    *self.pending_rename = Some(idx);
                }
            }

            let mut ops = SimpleOps {
                thumbnail_loads: &mut pending_thumbnail_loads,
                folder_scans: &mut pending_folder_scans,
                folder_preview_loads: &mut pending_folder_preview_loads,
                pending_rename: &mut pending_rename,
            };

            render_item_slot(ui, &mut ctx, &mut ops);
        }

        // Apply changes after render
        if let Some(new_text) = renaming_text_clone {
            if is_renaming {
                if let Some((_, ref mut text)) = self.renaming_state {
                    *text = new_text;
                }
            }
        }

        // Execute pending operations
        for path in pending_thumbnail_loads {
            ImageViewerApp::request_thumbnail_load(&*self, path);
        }

        for path in pending_folder_scans {
            ImageViewerApp::request_folder_scan(&*self, path);
        }

        for path in pending_folder_preview_loads {
            self.request_folder_preview_load(path);
        }

        if let Some(rename_idx) = pending_rename {
            self.rename_with_shell(rename_idx);
        }

        // Reset focus flag after first use
        if self.focus_rename {
            self.focus_rename = false;
        }
    }
}

impl mtt_file_manager::ui::components::item_slot::ItemSlotOperations for ImageViewerApp {
    fn request_thumbnail_load(&mut self, path: std::path::PathBuf) {
        // Call inherent method - uses &self so we need to reborrow
        ImageViewerApp::request_thumbnail_load(&*self, path);
    }

    fn request_folder_scan(&mut self, path: std::path::PathBuf) {
        // Call inherent method - uses &self so we need to reborrow
        ImageViewerApp::request_folder_scan(&*self, path);
    }

    fn request_folder_preview_load(&mut self, path: std::path::PathBuf) {
        self.request_folder_preview_load(path);
    }

    fn rename_item(&mut self, idx: usize) {
        self.rename_with_shell(idx);
    }
}

impl mtt_file_manager::ui::context_menu::ContextMenuOperations for ImageViewerApp {
    fn create_new_folder(&mut self) {
        self.create_new_folder();
    }

    fn command_copy(&mut self, idx: Option<usize>) {
        self.command_copy(idx);
    }

    fn command_cut(&mut self, idx: Option<usize>) {
        self.command_cut(idx);
    }

    fn command_paste(&mut self, idx: Option<usize>) {
        self.command_paste(idx);
    }

    fn rename_item(&mut self, idx: usize) {
        if let Some(item) = self.items.get(idx) {
            self.renaming_state = Some((idx, item.name.clone()));
            self.focus_rename = true;
        }
    }

    fn delete_with_shell(&mut self, idx: Option<usize>) {
        self.delete_with_shell_for_idx(idx);
    }
}

impl ImageViewerApp {
    /// Atualiza o thumbnail persistente do arquivo selecionado de forma que
    /// ele continue visível mesmo que o item saia do viewport (e seja removido do cache LRU).
    fn update_selected_thumbnail(&mut self) {
        if let Some(selected) = &self.selected_file {
            // Validate path exists before trying to load thumbnail
            if !selected.path.exists() {
                self.selected_file = None;
                self.selected_thumbnail = None;
                return;
            }
            
            // Tenta pegar do cache. Se não estiver lá, mantém None (será atualizado via message loop)
            if let Some(tex) = self.cache_manager.texture_cache.peek(&selected.path) {
                self.selected_thumbnail = Some(tex.clone());
            } else {
                // Se mudou de seleção e não tem no cache, limpa
                self.selected_thumbnail = None;
            }
        } else {
            self.selected_thumbnail = None;
        }
    }

    /// Limpa a seleção atual, o thumbnail persistente, metadados e a busca.
    /// Útil durante navegação entre pastas.
    fn reset_selection_and_search(&mut self) {
        self.selected_item = None;
        self.selected_file = None;
        self.selected_thumbnail = None;
        self.selected_metadata = None;
        self.search_query.clear();
        self.context_menu.target_path = None;
        self.renaming_state = None;
    }

    /// Resolve the target path for a context menu action.
    fn context_target_path(&self, item_idx: Option<usize>) -> Option<PathBuf> {
        if let Some(idx) = item_idx {
            return self.items.get(idx).map(|i| i.path.clone());
        }

        if let Some(p) = self.context_menu.target_path.clone() {
            return Some(p);
        }

        if let Some(sel) = &self.selected_file {
            return Some(sel.path.clone());
        }

        Some(PathBuf::from(&self.current_path))
    }

    /// Copy a filesystem path to the Windows clipboard as text.
    fn copy_path_to_clipboard(&self, path: &Path) {
        use clipboard_win::{formats, Clipboard, Setter};

        if let Ok(_clip) = Clipboard::new_attempts(10) {
            let _ = formats::Unicode.write_clipboard(&path.to_string_lossy());
        }
    }

    /// Create a Windows shell shortcut (.lnk) pointing to `target` in the same directory.
    fn create_shell_shortcut(&self, target: &Path) -> std::result::Result<PathBuf, String> {
        use windows::core::PCWSTR;
        use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
        use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

        let dest_dir = target
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(&self.current_path));

        let base_name = target
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| target.to_string_lossy().to_string());

        let mut candidate = dest_dir.join(format!("{} - Atalho.lnk", base_name));
        let mut counter = 2;
        while candidate.exists() {
            candidate = dest_dir.join(format!("{} - Atalho ({}).lnk", base_name, counter));
            counter += 1;
        }

        let result = unsafe {
            // SAFETY: COM is initialized for the current thread; errors are propagated as Strings.
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                .ok()
                .map_err(|e| format!("CoInitializeEx failed: {e}"))?;

            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| format!("CoCreateInstance ShellLink failed: {e}"))?;

            let wide_target: Vec<u16> = target
                .to_string_lossy()
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let wide_workdir: Vec<u16> = dest_dir
                .to_string_lossy()
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();

            link.SetPath(PCWSTR(wide_target.as_ptr()))
                .map_err(|e| format!("SetPath failed: {e}"))?;
            link.SetWorkingDirectory(PCWSTR(wide_workdir.as_ptr()))
                .map_err(|e| format!("SetWorkingDirectory failed: {e}"))?;

            let persist: IPersistFile = link
                .cast()
                .map_err(|e| format!("IPersistFile cast failed: {e}"))?;

            let wide_dest: Vec<u16> = candidate
                .to_string_lossy()
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            persist
                .Save(PCWSTR(wide_dest.as_ptr()), true)
                .map_err(|e| format!("Persist Save failed: {e}"))?;

            Ok(())
        };

        unsafe { CoUninitialize(); }

        result.map(|_| candidate)
    }

    fn populate_context_menu(&mut self, ctx: &egui::Context, path: &std::path::Path, is_empty_area: bool, _item_index: Option<usize>) {
        use mtt_file_manager::application::context_menu::ContextMenuItem;
        use mtt_file_manager::infrastructure::windows::native_menu::{extract_shell_menu, ShellMenuItem, is_known_verb};
        
        let mut items = Vec::new();
        
        // Special menu for Recycle Bin items
        if self.is_recycle_bin_view && !is_empty_area {
            // Menu items for recycle bin (no primary icons)
            items.push(ContextMenuItem::new(-52, "Restaurar").with_command("restore"));
            items.push(ContextMenuItem::new(-53, "Excluir permanentemente").with_command("delete_permanent"));
            items.push(ContextMenuItem::separator());
            items.push(ContextMenuItem::new(-28, "Propriedades").with_command("properties").with_shortcut("Alt+Enter"));
            
            self.context_menu.items = items;
            return;
        }
        
        // Special menu for empty area in Recycle Bin
        if self.is_recycle_bin_view && is_empty_area {
            items.push(ContextMenuItem::new(-54, "Esvaziar Lixeira").with_command("empty_recycle_bin"));
            self.context_menu.items = items;
            return;
        }
        
        // ========== PRIMARY ITEMS (Header bar) - matching Files ==========
        // These appear as icon buttons in the header
        items.push(ContextMenuItem::primary(-3, "Recortar").with_command("cut").with_shortcut("Ctrl+X"));
        items.push(ContextMenuItem::primary(-2, "Copiar").with_command("copy").with_shortcut("Ctrl+C"));
        
        let can_paste = self.clipboard_file.is_some() || mtt_file_manager::infrastructure::windows_clipboard::has_files_in_clipboard();
        items.push(ContextMenuItem::primary(-4, "Colar").with_command("paste").with_shortcut("Ctrl+V").enabled(can_paste));
        
        if !is_empty_area {
            items.push(ContextMenuItem::primary(-5, "Renomear").with_command("rename").with_shortcut("F2"));
            items.push(ContextMenuItem::primary(-6, "Excluir").with_command("delete").with_shortcut("Del"));
        }
        
        // ========== SECONDARY ITEMS (App-specific) ==========
        let can_paste = self.clipboard_file.is_some() || mtt_file_manager::infrastructure::windows_clipboard::has_files_in_clipboard();
        if is_empty_area {
            items.push(ContextMenuItem::separator());
            items.push(ContextMenuItem::new(-32, "Colar").with_command("paste").with_shortcut("Ctrl+V").enabled(can_paste));
            items.push(ContextMenuItem::new(-1, "Criar pasta").with_shortcut("Ctrl+Shift+N"));
        } else {
            items.push(ContextMenuItem::separator());
            items.push(ContextMenuItem::new(-20, "Abrir"));
            items.push(ContextMenuItem::new(-21, "Abrir em nova guia"));
            items.push(ContextMenuItem::separator());
            // Basic file operations as text items (in addition to header icons)
            items.push(ContextMenuItem::new(-30, "Recortar").with_command("cut").with_shortcut("Ctrl+X"));
            items.push(ContextMenuItem::new(-31, "Copiar").with_command("copy").with_shortcut("Ctrl+C"));
            items.push(ContextMenuItem::new(-32, "Colar").with_command("paste").with_shortcut("Ctrl+V").enabled(can_paste));
            items.push(ContextMenuItem::new(-33, "Renomear").with_command("rename").with_shortcut("F2"));
            items.push(ContextMenuItem::new(-34, "Excluir").with_command("delete").with_shortcut("Del"));
            items.push(ContextMenuItem::separator());
            items.push(ContextMenuItem::new(-24, "Copiar caminho").with_shortcut("Ctrl+Shift+C"));
            items.push(ContextMenuItem::new(-26, "Criar atalho"));
            items.push(ContextMenuItem::separator());
            items.push(ContextMenuItem::new(-28, "Propriedades").with_command("properties").with_shortcut("Alt+Enter"));
        }
        
        // ========== SHELL ITEMS (Third-party extensions) ==========
        if let Some(hwnd) = self.native_hwnd {
            if let Ok(shell_ctx) = extract_shell_menu(hwnd, path) {
                // Convert Shell items to UI items, filtering known verbs
                fn convert(ui_ctx: &egui::Context, shell_item: &ShellMenuItem) -> Option<ContextMenuItem> {
                    // Filter items we handle internally
                    if let Some(ref verb) = shell_item.command_string {
                        if is_known_verb(verb) {
                            return None;
                        }
                    }
                    
                    // Fallback text-based filter for localized or verbless items
                    let lower_text = shell_item.text.to_lowercase();
                    let blacklisted_texts = [
                        "pin to quick access", "fixar no acesso rápido",
                        "restore previous versions", "restaurar versões anteriores",
                        "copy as path", "copiar como caminho",
                        "create shortcut", "criar atalho"
                    ];
                    if blacklisted_texts.iter().any(|&t| lower_text.contains(t)) {
                        return None;
                    }
                    
                    // Resize icon to 16x16 if needed
                    let icon = shell_item.icon_rgba.as_ref().map(|(rgba, w, h)| {
                        let (final_rgba, fw, fh) = if *w != 16 || *h != 16 {
                            // Simple resize - in production would use proper resampling
                            (rgba.clone(), *w, *h)
                        } else {
                            (rgba.clone(), *w, *h)
                        };
                        let color_image = egui::ColorImage::from_rgba_unmultiplied([fw as usize, fh as usize], &final_rgba);
                        ui_ctx.load_texture(
                            format!("menu_icon_{}", shell_item.id),
                            color_image,
                            Default::default()
                        )
                    });
                    
                    let sub_items: Vec<ContextMenuItem> = shell_item.sub_items.iter()
                        .filter_map(|s| convert(ui_ctx, s))
                        .collect();
                    
                    Some(ContextMenuItem {
                        id: shell_item.id as i32,
                        text: shell_item.text.clone(),
                        icon,
                        sub_items,
                        is_separator: shell_item.is_separator,
                        is_enabled: shell_item.is_enabled,
                        is_primary: false,
                        keyboard_shortcut: None,
                        command_string: shell_item.command_string.clone(),
                        show_in_overflow: false,
                        has_pending_submenu: shell_item.pending_submenu_handle.is_some(),
                    })
                }
                
                let shell_items: Vec<ContextMenuItem> = shell_ctx.items.iter()
                    .filter_map(|s| convert(ctx, s))
                    .collect();
                
                // Separate shell items: common ones visible, rest go to overflow
                let mut visible_shell_items = Vec::new();
                let mut overflow_shell_items = Vec::new();
                
                for s_item in shell_items {
                    // Keep items with submenus OR pending submenus (like 7-Zip, WinRAR) visible
                    if !s_item.sub_items.is_empty() || s_item.has_pending_submenu {
                        visible_shell_items.push(s_item);
                    } else if !s_item.is_separator {
                        overflow_shell_items.push(s_item);
                    }
                }
                
                // Add visible shell items (with submenus like 7-Zip)
                if !visible_shell_items.is_empty() {
                    items.push(ContextMenuItem::separator());
                    for s_item in visible_shell_items {
                        items.push(s_item);
                    }
                }
                
                // Add overflow submenu with remaining shell items
                if !overflow_shell_items.is_empty() {
                    items.push(ContextMenuItem::separator());
                    items.push(ContextMenuItem::new(-99, "Mostrar mais opções")
                        .with_subitems(overflow_shell_items));
                }
                
                // Keep the native context alive for command invocation
                self.context_menu.native_context = Some(std::rc::Rc::new(shell_ctx));
            }
        }
        
        self.context_menu.items = items;
    }
}

impl eframe::App for ImageViewerApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // Validate selected_file on first frame - clear if path no longer exists
        if self.startup_tick == 0 {
            if let Some(ref file) = self.selected_file {
                if !file.path.exists() {
                    self.selected_file = None;
                    self.selected_thumbnail = None;
                    self.selected_metadata = None;
                }
            }
        }
        
        // --- 3-STAGE STARTUP SEQUENCE ---
        // Stage 1 (frame 1): Apply saved geometry (maximize OR size) while hidden
        // Stage 2 (frames 2-5): Wait for layouts to stabilize  
        // Stage 3 (frame 5): Reveal window
        if self.startup_tick < 5 {
            self.startup_tick += 1;
            
            if self.startup_tick == 1 {
                // Frame 1: Apply saved geometry while window is hidden
                if self.saved_is_maximized {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
                } else {
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(
                        egui::Vec2::new(self.saved_window_width, self.saved_window_height)
                    ));
                }
            }
            
            if self.startup_tick == 5 {
                // Frame 5: Reveal the window
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);

                // FINAL INITIALIZATION: Agora que a UI estÃ¡ pronta, garante que a aba inicial estÃ¡ populada
                if self.is_computer_view {
                    self.setup_computer_view();
                } else {
                    self.load_folder(false);
                }
                self.sync_to_tab();
            }
            
            // Keep the loop running fast during startup
            ctx.request_repaint();
        }
        // --- END STARTUP SEQUENCE ---

        // Track current window state for saving on exit
        let (size_changed, maximized_changed) = ctx.input(|i| {
            let mut size_changed = false;
            let mut maximized_changed = false;
            
            if let Some(rect) = i.viewport().inner_rect {
                // Only save size when NOT maximized
                if !i.viewport().maximized.unwrap_or(false) {
                    if (self.saved_window_width - rect.width()).abs() > 1.0 || 
                       (self.saved_window_height - rect.height()).abs() > 1.0 {
                        size_changed = true;
                    }
                    self.saved_window_width = rect.width();
                    self.saved_window_height = rect.height();
                }
            }
            
            let new_maximized = i.viewport().maximized.unwrap_or(false);
            if new_maximized != self.saved_is_maximized {
                maximized_changed = true;
            }
            self.saved_is_maximized = new_maximized;
            
            (size_changed, maximized_changed)
        });
        
        // Save preferences when window state changes
        if size_changed || maximized_changed {
            self.save_preferences();
        }
        // --- END STARTUP SEQUENCE ---

        self.ensure_window_handle(frame);

        // --- DETECÇÃO DE COMANDOS DE SISTEMA (Clipboard) ---
        // Usa detecção via eventos RAW de teclas.
        // Só bloqueia durante renomeação ou edição de endereço.

        // DEBUG: Log todos os frames para verificar se o código está rodando
        // eprintln!("[DEBUG] Frame update - renaming={:?} address_editing={}", self.renaming_state.is_some(), self.is_address_editing);

        if self.renaming_state.is_none() && !self.is_address_editing {
            // Detectar teclas através dos eventos (Key events)
            let mut do_copy = false;
            let mut do_cut = false;
            let mut do_paste = false;

            ctx.input(|i| {
                // Log all key events to see what's arriving
                for event in &i.events {
                    match event {
                        egui::Event::Key {
                            key,
                            pressed,
                            modifiers,
                            ..
                        } => {
                            if *pressed && modifiers.ctrl {
                                eprintln!("[DEBUG] Key event: {:?} Ctrl+pressed", key);
                                match key {
                                    egui::Key::C => do_copy = true,
                                    egui::Key::X => do_cut = true,
                                    egui::Key::V => do_paste = true,
                                    // TAB MANAGEMENT SHORTCUTS
                                    egui::Key::T => {
                                        // Ctrl+T = New tab
                                        self.sync_to_tab();
                                        self.tab_manager.new_tab();
                                        self.sync_from_tab();
                                        self.setup_computer_view();
                                        self.sync_to_tab();
                                    }
                                    egui::Key::W => {
                                        // Ctrl+W = Close current tab
                                        if self.tab_manager.close_active_tab() {
                                            // Last tab - quit app
                                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                                        } else {
                                            self.sync_from_tab();
                                        }
                                    }
                                    egui::Key::Tab => {
                                        // Ctrl+Tab = Next tab, Ctrl+Shift+Tab = Previous tab
                                        self.sync_to_tab();
                                        if modifiers.shift {
                                            self.tab_manager.prev_tab();
                                        } else {
                                            self.tab_manager.next_tab();
                                        }
                                        self.sync_from_tab();
                                    }
                                    _ => {}
                                }
                            }
                        }
                        egui::Event::Copy => {
                            do_copy = true;
                        }
                        egui::Event::Cut => {
                            do_cut = true;
                        }
                        egui::Event::Paste(_) => {
                            do_paste = true;
                        }
                        _ => {}
                    }
                }
            });

            // Fallback: use Windows GetAsyncKeyState for hardware-level detection
            // (Windows consumes Ctrl+V key events when clipboard has files)
            // VK_CONTROL = 0x11, VK_V = 0x56
            let ctrl_down = unsafe { GetAsyncKeyState(0x11) < 0 };
            let v_down = unsafe { GetAsyncKeyState(0x56) < 0 };

            // Debounced paste detection (only fire once per key press)
            if ctrl_down && v_down && !self.paste_key_debounce {
                do_paste = true;
                self.paste_key_debounce = true;
            } else if !v_down {
                self.paste_key_debounce = false;
            }

            // Executar ações de clipboard
            if do_copy {
                self.command_copy(None);
            }
            if do_cut {
                self.command_cut(None);
            }
            if do_paste {
                self.command_paste(None);
            }

            // Delete: Excluir
            if ctx.input(|i| i.key_pressed(egui::Key::Delete)) {
                self.delete_with_shell_for_idx(None);
            }

            // Ctrl + Shift + N: Nova Pasta
            if ctx.input(|i| i.modifiers.ctrl && i.modifiers.shift && i.key_pressed(egui::Key::N)) {
                self.create_new_folder();
            }
        } else {
            // Durante renomeação: ESC cancela a operação
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.renaming_state = None;
                self.focus_rename = false;
            }
        }

        self.process_incoming_messages(ctx);
        self.refresh_drives_if_needed();
        self.ensure_folder_icon(ctx);
        self.ensure_computer_icon(ctx);

        // Status Bar (Footer) - Definido primeiro para ocupar toda a largura
        egui::TopBottomPanel::bottom("status_bar")
            .exact_height(24.0)
            .show(ctx, |ui| {
                use mtt_file_manager::ui::status_bar::{render_status_bar, StatusBarAction};
                let action = render_status_bar(
                    ui,
                    &mut self.is_loading_folder,
                    self.total_items,
                    &mut self.view_mode,
                    &mut self.thumbnail_size,
                    &mut self.sort_mode,
                    &mut self.sort_descending,
                    &mut self.folders_position,
                    &self.cache_manager.texture_cache,
                );
                match action {
                    StatusBarAction::SortChanged => {
                        self.sort_items();
                        self.save_preferences();
                    }
                    StatusBarAction::ViewModeChanged => {
                        // View mode changed - nothing extra to do
                    }
                    StatusBarAction::None => {}
                }
            });

        // Windows 11 style sidebar
        // Left Sidebar moved to after TopPanels for correct layout

        // TAB BAR (custom title bar with tabs and window controls)
        egui::TopBottomPanel::top("tab_bar_panel")
            .show_separator_line(false)
            .exact_height(36.0)
            .frame(egui::Frame {
                fill: if ctx.style().visuals.dark_mode {
                    egui::Color32::from_rgb(32, 32, 32)
                } else {
                    egui::Color32::from_rgb(243, 243, 243)
                },
                ..Default::default()
            })
            .show(ctx, |ui| {
                use mtt_file_manager::ui::tab_bar::{render_tab_bar, TabBarAction};
                let action = render_tab_bar(
                    ui,
                    &self.tab_manager,
                    &mut self.svg_icon_manager,
                    frame,
                    self.cache_manager.computer_icon.as_ref(), // Pass native computer icon
                    &mut self.item_icon_loader,               // Pass icon loader for dynamic icons
                );
                
                match action {
                    TabBarAction::SwitchTab(idx) => {
                        self.sync_to_tab();
                        self.tab_manager.switch_to(idx);
                        self.sync_from_tab();
                    }
                    TabBarAction::NewTab => {
                        self.sync_to_tab();
                        self.tab_manager.new_tab();
                        self.sync_from_tab();
                        self.setup_computer_view(); // Popula os drives na nova aba
                        self.sync_to_tab(); // Salva estado populado
                    }
                    TabBarAction::CloseTab(idx) => {
                        if self.tab_manager.close_tab(idx) {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        } else {
                            self.sync_from_tab();
                        }
                    }
                    TabBarAction::CloseApp => {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    TabBarAction::ToggleMaximize => {
                        let is_maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!is_maximized));
                    }
                    TabBarAction::Minimize => {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                    }
                    TabBarAction::None => {}
                }
            });

        // Top navigation bar
        egui::TopBottomPanel::top("nav_bar")
            .show_separator_line(true)
            .frame(egui::Frame {
                fill: if ctx.style().visuals.dark_mode {
                    egui::Color32::from_rgb(45, 45, 45) // Matches tab_bar.rs active_bg
                } else {
                    egui::Color32::from_rgb(255, 255, 255) // Matches tab_bar.rs active_bg
                },
                ..Default::default()
            })
            .show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.style_mut().spacing.item_spacing.x = 8.0;

                // 1. NAVEGAÇÃO (ESQUERDA) - Bloqueados durante renomeação
                let is_renaming = self.renaming_state.is_some();

                let can_back = self.can_go_back() && !is_renaming;
                if self.icon_button(ui, ICON_ARROW_LEFT, "Voltar").clicked() && can_back {
                    self.go_back();
                }

                let can_forward = self.can_go_forward() && !is_renaming;
                if self.icon_button(ui, ICON_ARROW_RIGHT, "Avançar").clicked() && can_forward {
                    self.go_forward();
                }

                if self
                    .icon_button(ui, ICON_ARROW_UP, "Subir um nível")
                    .clicked()
                    && !is_renaming
                {
                    self.go_up_one_level();
                }

                if self.icon_button(ui, ICON_REFRESH, "Recarregar").clicked() && !is_renaming {
                    self.trigger_manual_refresh();
                }

                ui.separator();

                // Botão de Nova Pasta
                if self.icon_button(ui, ICON_FOLDER_ADD, "Criar Nova Pasta (Ctrl+Shift+N)").clicked() && !is_renaming {
                    self.create_new_folder();
                }

                ui.separator();

                if self.icon_button(ui, ICON_HOME, "Home").clicked() && !is_renaming {
                    self.navigate_to_computer();
                }

                // 2. ELEMENTOS DA DIREITA (DIREITA -> ESQUERDA)
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(4.0);

                    // Zoom
                    ui.add_sized(
                        egui::vec2(80.0, 20.0),
                        egui::Slider::new(&mut self.thumbnail_size, 64.0..=256.0).show_value(false),
                    );
                    ui.label("Zoom");

                    ui.separator();

                    // Detalhes (Antigo Preview)
                    if self
                        .toggle_icon_button(ui, ICON_DETAILS, self.show_preview_panel, "Detalhes")
                        .clicked()
                    {
                        self.show_preview_panel = !self.show_preview_panel;
                    }

                    ui.separator();

                    // Modo de Visualização
                    if self
                        .toggle_icon_button(
                            ui,
                            ICON_LIST,
                            self.view_mode == ViewMode::List,
                            "Lista",
                        )
                        .clicked()
                    {
                        self.view_mode = ViewMode::List;
                    }
                    if self
                        .toggle_icon_button(
                            ui,
                            ICON_GRID,
                            self.view_mode == ViewMode::Grid,
                            "Grade",
                        )
                        .clicked()
                    {
                        self.view_mode = ViewMode::Grid;
                    }

                    ui.separator();

                    // Ordenação
                    let sort_symbol = if self.sort_descending { "↓" } else { "↑" };
                    if ui
                        .button(sort_symbol)
                        .on_hover_text("Inverter Ordem")
                        .clicked()
                    {
                        self.sort_descending = !self.sort_descending;
                        self.sort_items();
                        self.save_preferences();
                    }

                    egui::ComboBox::from_id_salt("sort_mode")
                        .selected_text(match self.sort_mode {
                            SortMode::Name => "Nome",
                            SortMode::Date => "Data",
                            SortMode::Size => "Tamanho",
                            SortMode::Type => "Tipo",
                        })
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_value(&mut self.sort_mode, SortMode::Name, "Nome")
                                .clicked()
                            {
                                self.sort_items();
                                self.save_preferences();
                            }
                            if ui
                                .selectable_value(&mut self.sort_mode, SortMode::Date, "Data")
                                .clicked()
                            {
                                self.sort_items();
                                self.save_preferences();
                            }
                            if ui
                                .selectable_value(&mut self.sort_mode, SortMode::Size, "Tamanho")
                                .clicked()
                            {
                                self.sort_items();
                                self.save_preferences();
                            }
                            if ui
                                .selectable_value(&mut self.sort_mode, SortMode::Type, "Tipo")
                                .clicked()
                            {
                                self.sort_items();
                                self.save_preferences();
                            }
                        });

                    ui.separator();

                    // Busca
                    let search_width = 120.0;
                    let search_response = ui.add_sized(
                        egui::vec2(search_width, 22.0),
                        egui::TextEdit::singleline(&mut self.search_query).hint_text("Buscar..."),
                    );
                    if search_response.changed() {
                        self.filter_items();
                    }
                    mtt_file_manager::ui::svg_icons::icon_image(
                        ui,
                        &mut self.svg_icon_manager,
                        "search",
                        16.0,
                    );

                    ui.separator();

                    // 3. BARRA DE ENDEREÇO (Breadcrumbs ou Edição)
                    // No layout reverse (right_to_left), o available_width() retorna o que sobrou à esquerda.
                    let addr_width = (ui.available_width() - 4.0).max(100.0);
                    let (addr_rect, _addr_response) =
                        ui.allocate_exact_size(egui::vec2(addr_width, 24.0), egui::Sense::hover());

                    let mut navigate_target = None;
                    let mut start_editing = false;

                    // IMPORTANTE: Usar allocate_new_ui com closure para ter o novo Ui com layout correto
                    ui.allocate_new_ui(
                        egui::UiBuilder::new()
                            .max_rect(addr_rect)
                            .layout(egui::Layout::left_to_right(egui::Align::Center)),
                        |ui| {
                            if self.is_address_editing {
                                let edit_response = ui.add_sized(
                                    ui.available_size(),
                                    egui::TextEdit::singleline(&mut self.path_input)
                                        .hint_text("Caminho...")
                                        .id_source("address_edit"),
                                );

                                if edit_response.clicked_elsewhere()
                                    || (edit_response.lost_focus()
                                        && !ui.input(|i| i.key_pressed(egui::Key::Enter)))
                                {
                                    self.is_address_editing = false;
                                }

                                if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                    let path = self.path_input.clone();
                                    if Path::new(&path).exists() {
                                        navigate_target = Some(path);
                                        self.is_address_editing = false;
                                    } else {
                                        self.path_input = self.current_path.clone();
                                        self.is_address_editing = false;
                                    }
                                }

                                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                    self.is_address_editing = false;
                                    self.path_input = self.current_path.clone();
                                }
                            } else {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 2.0;

                                    if self.current_path == "Este Computador" {
                                        ui.label(egui::RichText::new("Este Computador").size(13.0));
                                    } else {
                                        let path = Path::new(&self.current_path);
                                        let mut full_accumulated = PathBuf::new();
                                        let components: Vec<_> = path.components().collect();

                                        for (i, comp) in components.iter().enumerate() {
                                            let comp_str = comp.as_os_str().to_string_lossy();
                                            let display_name = comp_str.trim_end_matches('\\');

                                            if display_name.is_empty() && i > 0 {
                                                continue;
                                            }

                                            full_accumulated.push(comp);
                                            // Normaliza drive roots: "Z:" -> "Z:\" para navegação correta
                                            let target_path = {
                                                let p =
                                                    full_accumulated.to_string_lossy().to_string();
                                                if p.len() == 2 && p.ends_with(':') {
                                                    format!("{}\\", p)
                                                } else {
                                                    p
                                                }
                                            };

                                            // Nome do drive ou pasta
                                            let display = if display_name.is_empty() {
                                                comp_str.into_owned() // Root / ou C:\
                                            } else {
                                                display_name.to_string()
                                            };

                                            if ui.button(display).clicked() {
                                                navigate_target = Some(target_path);
                                            }

                                            if i < components.len() - 1 {
                                                ui.label(
                                                    egui::RichText::new("›")
                                                        .size(14.0)
                                                        .color(egui::Color32::from_gray(120)),
                                                );
                                            }
                                        }
                                    }

                                    // Espaço clicável à direita para entrar no modo edição
                                    let remaining = ui.available_width();
                                    if remaining > 0.0 {
                                        let (_rect, resp) = ui.allocate_exact_size(
                                            egui::vec2(remaining, ui.available_height()),
                                            egui::Sense::click(),
                                        );
                                        if resp.clicked() {
                                            start_editing = true;
                                        }
                                    }
                                });
                            }
                        },
                    );

                    if let Some(target) = navigate_target {
                        self.navigate_to(&target);
                    }
                    if start_editing {
                        self.path_input = self.current_path.clone();
                        self.is_address_editing = true;
                        ui.ctx().memory_mut(|m| {
                            m.request_focus(egui::Id::from("address_edit").with("text_edit"))
                        });
                    }
                });
            });
            ui.add_space(4.0);
        });

        // Windows 11 style sidebar (Restored)
        
        let sidebar_response = egui::SidePanel::left("sidebar")
            .min_width(150.0)
            .default_width(self.sidebar_left_width.max(150.0)) // Garante que nunca seja 0
            .resizable(true)
            .show(ctx, |ui| {
                use mtt_file_manager::ui::sidebar::{render_sidebar, SidebarContext};

                // Clonar dados necessários para evitar problemas de borrow
                let disks = self.disks.clone();
                let current_path = self.current_path.clone();
                let is_computer_view = self.is_computer_view;
                let computer_icon = self.cache_manager.computer_icon.clone();

                // Criar contexto para sidebar
                let mut ctx = SidebarContext {
                    disks: &disks,
                    current_path: &current_path,
                    is_computer_view,
                    is_recycle_bin_view: self.is_recycle_bin_view,
                    computer_icon: computer_icon.as_ref(),
                    is_renaming: self.renaming_state.is_some(),
                    icon_loader: &mut self.item_icon_loader,
                    onedrive_path: self.onedrive_path.as_deref(),
                    onedrive_icon: self.onedrive_icon.as_ref(),
                };

                render_sidebar(ui, &mut ctx)
            });
        
        // Captura a largura REAL do painel (não a disponível dentro dele)
        // IMPORTANTE: Não atualiza se janela está minimizada (rect fica inválido)
        let is_minimized = ctx.input(|i| i.viewport().minimized.unwrap_or(false));
        let actual_panel_width = sidebar_response.response.rect.width();
        if !is_minimized && actual_panel_width > 100.0 && (self.sidebar_left_width - actual_panel_width).abs() > 2.0 {
            self.sidebar_left_width = actual_panel_width;
        }
        
        let sidebar_action = sidebar_response.inner;

        // Processar ação da sidebar (após ctx ser dropado e self liberado)
        if let Some(action) = sidebar_action {
            use mtt_file_manager::ui::sidebar::SidebarAction;
            match action {
                SidebarAction::NavigateTo(path) => self.navigate_to(&path),
                SidebarAction::NavigateToComputer => self.navigate_to_computer(),
                SidebarAction::NavigateToRecycleBin => self.navigate_to_recycle_bin(),
            }
        }

        // Preview Pane (Windows Explorer style) - ANTES do CentralPanel
        if self.show_preview_panel {
            self.refresh_selected_metadata();
            
            let right_panel_response = egui::SidePanel::right("preview_panel")
                .resizable(true)
                .default_width(self.sidebar_right_width.max(250.0)) // Garante que nunca seja 0
                .min_width(250.0)
                .max_width(500.0)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .id_source("preview_scroll")
                        .show(ui, |ui| {
                            ui.set_max_width(ui.available_width());
                            let effective_file = if let Some(file) = self.selected_file.clone() {
                                // Na lixeira, não verificar se o path existe (pois usamos paths virtuais)
                                if self.is_recycle_bin_view || file.path.exists() {
                                    Some(file)
                                } else {
                                    // File no longer exists - clear selection
                                    None
                                }
                            } else if self.is_recycle_bin_view {
                                // Na lixeira sem seleção, mostra info da Lixeira
                                let entry = FileEntry {
                                    path: PathBuf::from("Lixeira"),
                                    name: "Lixeira".to_string(),
                                    is_dir: true,
                                    size: 0,
                                    modified: 0,
                                    folder_cover: None,
                                    drive_info: None,
                                    sync_status: mtt_file_manager::domain::file_entry::SyncStatus::None,
                                    deletion_date: None,
                                };
                                Some(entry)
                            } else if !self.is_computer_view {
                                // Fallback: mostra informações da pasta ou drive atual
                                let path = std::path::PathBuf::from(&self.current_path);
                                let mut entry = FileEntry::from_path(path.clone(), true);
                                
                                // Verifica se é o root de um drive (ex: C:\)
                                if path.to_string_lossy().len() <= 3 && path.to_string_lossy().contains(':') {
                                    use mtt_file_manager::infrastructure::windows::get_volume_info;
                                    let vol = get_volume_info(&self.current_path);
                                    let drive_type = windows_infra::detect_drive_type(&self.current_path);
                                    
                                    let label = self.disks.iter()
                                        .find(|(p, _)| p.starts_with(&self.current_path) || self.current_path.starts_with(p))
                                        .map(|(_, l)| l.clone())
                                        .unwrap_or_else(|| self.current_path.clone());
                                        
                                    entry.name = label;
                                    entry.drive_info = Some(mtt_file_manager::domain::file_entry::DriveInfo {
                                        file_system: vol.file_system,
                                        total_space: vol.total_space,
                                        free_space: vol.free_space,
                                        drive_type,
                                    });
                                } else {
                                    entry.name = path.file_name()
                                        .map(|n| n.to_string_lossy().to_string())
                                        .unwrap_or_else(|| self.current_path.clone());
                                }
                                Some(entry)
                            } else {
                                None
                            };

                            if let Some(file) = effective_file {
                                ui.heading("Detalhes");
                                ui.separator();

                                // Preview de imagem/video (se houver thumbnail)
                                let _has_thumbnail =
                                    self.cache_manager.texture_cache.peek(&file.path).is_some();
                                // Detecta se é mídia usando Windows Perceived Type API
                                let is_media = file
                            .path
                            .extension()
                            .map(|ext: &std::ffi::OsStr| {
                                mtt_file_manager::infrastructure::windows::is_media_extension(
                                    &ext.to_string_lossy(),
                                )
                            })
                            .unwrap_or(false);

                                let texture = if let Some(tex) = &self.selected_thumbnail {
                                    Some(tex.clone())
                                } else {
                                    self.cache_manager.texture_cache.peek(&file.path).cloned()
                                };

                                if let (Some(tex), true) = (texture, is_media) {
                                    // Mostra thumbnail de imagem/video
                                    let max_preview_width = ui.available_width() - 8.0;
                                    let max_preview_size =
                                        egui::vec2(max_preview_width, max_preview_width);

                                    ui.vertical_centered(|ui| {
                                        ui.add(
                                            egui::Image::new(&tex)
                                                .max_size(max_preview_size)
                                                .shrink_to_fit(),
                                        );
                                    });
                                    
                                    // Botão de recarregar thumbnail (centralizado)
                                    ui.vertical_centered(|ui| {
                                        if self.icon_button(ui, ICON_REFRESH, "Recarregar Thumbnail").clicked() {
                                            // 1. Remove do cache SQLite
                                            self.disk_cache.remove_cache_for_path(&file.path);
                                            
                                            // 2. Remove do cache RAM (texture_cache)
                                            self.cache_manager.texture_cache.pop(&file.path);
                                            
                                            // 3. Remove do loading_set para permitir re-carregamento
                                            self.cache_manager.loading_set.remove(&file.path);
                                            
                                            // 4. Dispara re-extração normal via worker pool
                                            // O worker já tenta múltiplas abordagens, incluindo Shell API
                                            let _ = self.thumbnail_req_sender.send((file.path.clone(), self.generation));
                                            
                                            // Notifica o usuário
                                            self.notifications.push(
                                                mtt_file_manager::application::AppNotification::info(
                                                    "Recarregando thumbnail...".to_string(),
                                                ),
                                            );
                                        }
                                    });
                                    
                                    ui.separator();
                                } else {
                                    // Pasta ou Drive ou Arquivo sem Thumbnail
                                    let max_w: f32 = ui.available_width() - 40.0;
                                    let icon_size: f32 = (120.0f32).min(max_w);

                                    ui.vertical_centered(|ui| {
                                        ui.add_space(20.0);
                                        if let Some(_) = &file.drive_info {
                                            // DRIVE
                                            if let Some(icon) =
                                                self.item_icon_loader.get_or_load_drive_icon(
                                                    ui.ctx(),
                                                    &file.path.to_string_lossy(),
                                                )
                                            {
                                                ui.add(
                                                    egui::Image::new(&icon)
                                                        .max_size(egui::vec2(icon_size, icon_size)),
                                                );
                                            } else {
                                                ui.label(
                                                    egui::RichText::new("??").size(icon_size * 0.8),
                                                );
                                            }
                                        } else if self.is_recycle_bin_view && file.name == "Lixeira" {
                                            // LIXEIRA - mostra ícone da lixeira
                                            if let Some(icon) = self.item_icon_loader.ensure_recycle_bin_icon(ui.ctx()) {
                                                ui.add(
                                                    egui::Image::new(&icon)
                                                        .max_size(egui::vec2(icon_size, icon_size)),
                                                );
                                            } else {
                                                ui.label(
                                                    egui::RichText::new("🗑").size(icon_size * 0.6),
                                                );
                                            }
                                        } else if file.is_dir {
                                            // PASTA (Usa preview nativo do Windows - sandwich effect)
                                            // Na lixeira, não tentar carregar preview de pastas
                                            if self.is_recycle_bin_view {
                                                // Pasta na lixeira - mostra ícone de pasta genérico
                                                self.item_icon_loader.ensure_folder_icon(ui.ctx());
                                                if let Some(icon) = self.item_icon_loader.folder_icon() {
                                                    ui.add(
                                                        egui::Image::new(icon)
                                                            .max_size(egui::vec2(icon_size, icon_size)),
                                                    );
                                                } else {
                                                    ui.label(
                                                        egui::RichText::new("📁").size(icon_size * 0.6),
                                                    );
                                                }
                                            } else {
                                            let folder_rect = ui
                                                .allocate_exact_size(
                                                    egui::vec2(icon_size, icon_size),
                                                    egui::Sense::hover(),
                                                )
                                                .0;

                                            // Tenta usar o preview nativo (Shell Sandwich)
                                            let native_preview = self.cache_manager.folder_preview_cache.get(&file.path).cloned();
                                            let is_loading = self.cache_manager.folder_preview_loading.contains(&file.path);

                                            if let Some(tex) = native_preview {
                                                // Preview nativo carregado - desenha mantendo aspect ratio
                                                let tex_size = tex.size_vec2();
                                                let aspect = tex_size.x / tex_size.y;
                                                
                                                let (draw_w, draw_h) = if aspect > 1.0 {
                                                    (folder_rect.width(), folder_rect.width() / aspect)
                                                } else {
                                                    (folder_rect.height() * aspect, folder_rect.height())
                                                };
                                                
                                                let offset_x = (folder_rect.width() - draw_w) / 2.0;
                                                let offset_y = (folder_rect.height() - draw_h) / 2.0;
                                                let draw_rect = egui::Rect::from_min_size(
                                                    folder_rect.min + egui::vec2(offset_x, offset_y),
                                                    egui::vec2(draw_w, draw_h),
                                                );
                                                
                                                ui.painter().image(
                                                    tex.id(),
                                                    draw_rect,
                                                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                                    egui::Color32::WHITE,
                                                );
                                            } else if is_loading {
                                                // Spinner enquanto carrega
                                                ui.painter().rect_filled(
                                                    folder_rect,
                                                    4.0,
                                                    egui::Color32::from_gray(245),
                                                );
                                                
                                                let spinner_size = folder_rect.width().min(folder_rect.height()) * 0.3;
                                                let center = folder_rect.center();
                                                let radius = spinner_size / 2.0 - 2.0;
                                                let time = ui.input(|i| i.time);
                                                let angle = (time * 3.0) as f32;
                                                let stroke = egui::Stroke::new(3.0, egui::Color32::from_rgb(100, 150, 220));
                                                
                                                let points: Vec<egui::Pos2> = (0..20)
                                                    .map(|i| {
                                                        let t = i as f32 / 19.0 * std::f32::consts::PI * 1.5;
                                                        let a = angle + t;
                                                        egui::pos2(center.x + radius * a.cos(), center.y + radius * a.sin())
                                                    })
                                                    .collect();
                                                
                                                ui.painter().add(egui::Shape::line(points, stroke));
                                                ui.ctx().request_repaint();
                                            } else {
                                                // Não tem preview e não está carregando - dispara carregamento
                                                if self.cache_manager.folder_preview_loading.len() < 30 {
                                                    self.cache_manager.folder_preview_loading.insert(file.path.clone());
                                                    let _ = self.folder_preview_sender.send(file.path.clone());
                                                }
                                                
                                                // Mostra placeholder enquanto inicia
                                                ui.painter().rect_filled(
                                                    folder_rect,
                                                    4.0,
                                                    egui::Color32::from_gray(240),
                                                );
                                                ui.painter().text(
                                                    folder_rect.center(),
                                                    egui::Align2::CENTER_CENTER,
                                                    "📁",
                                                    egui::FontId::proportional(icon_size * 0.4),
                                                    egui::Color32::from_gray(180),
                                                );
                                            }
                                            } // fecha else !is_recycle_bin_view
                                        } else {
                                            // ARQUIVO SEM THUMBNAIL
                                            // Na lixeira ou quando o arquivo não existe, use ícone por extensão
                                            let icon_opt = if self.is_recycle_bin_view || !file.path.exists() {
                                                let ext_str = file
                                                    .name
                                                    .rsplit_once('.')
                                                    .map(|(_, ext)| format!(".{}", ext))
                                                    .unwrap_or_else(|| ".bin".to_string());
                                                mtt_file_manager::infrastructure::windows::get_file_type_icon(
                                                    false,
                                                    &ext_str,
                                                    IconSize::Large,
                                                )
                                                .ok()
                                                .and_then(|(rgba_data, w, h)| {
                                                    Some(ui.ctx().load_texture(
                                                        format!("icon_{}", ext_str),
                                                        egui::ColorImage::from_rgba_unmultiplied(
                                                            [w as usize, h as usize],
                                                            &rgba_data,
                                                        ),
                                                        egui::TextureOptions::NEAREST,
                                                    ))
                                                })
                                            } else {
                                                self.get_or_load_icon(ui.ctx(), &file.path)
                                            };

                                            if let Some(icon) = icon_opt {
                                                ui.add(egui::Image::new(&icon).max_size(
                                                    egui::vec2(icon_size * 0.6, icon_size * 0.6),
                                                ));
                                            } else {
                                                ui.label(
                                                    egui::RichText::new("??").size(icon_size * 0.6),
                                                );
                                            }
                                        }
                                        ui.add_space(20.0);
                                    });
                                    ui.separator();
                                }

                                // Tabela de detalhes (Manual Responsive Grid)
                                let selected_metadata =
                                    self.selected_metadata.as_ref().and_then(|(p, meta)| {
                                        if p == &file.path {
                                            Some(meta)
                                        } else {
                                            None
                                        }
                                    });
                                let is_loading_meta = self.metadata_loading.contains(&file.path);

                                let key_w = 110.0;
                                let mut add_detail =
                                    |ui: &mut egui::Ui, label: &str, value: String| {
                                        ui.horizontal_top(|ui| {
                                            ui.add_sized(
                                                egui::vec2(key_w, 0.0),
                                                egui::Label::new(
                                                    egui::RichText::new(label).strong(),
                                                ),
                                            );
                                            ui.add(egui::Label::new(value).wrap());
                                        });
                                        ui.add_space(2.0);
                                    };

                                ui.scope(|ui| {
                                    ui.set_max_width(ui.available_width());

                                    if let Some(drive) = &file.drive_info {
                                        add_detail(
                                            ui,
                                            "Tipo:",
                                            drive.drive_type.label().to_string(),
                                        );

                                        let used_space = drive.total_space - drive.free_space;
                                        let usage_percent = if drive.total_space > 0 {
                                            (used_space as f64 / drive.total_space as f64) * 100.0
                                        } else {
                                            0.0
                                        };

                                        add_detail(ui, "Uso:", format!("{:.0}%", usage_percent));
                                        add_detail(ui, "Livre:", format_size(drive.free_space));
                                        add_detail(ui, "Total:", format_size(drive.total_space));
                                        add_detail(
                                            ui,
                                            "Sist. Arq:",
                                            if drive.file_system.is_empty() {
                                                "NTFS".to_string()
                                            } else {
                                                drive.file_system.clone()
                                            },
                                        );
                                        add_detail(ui, "BitLocker:", "Desligado".to_string());
                                    } else {
                                        add_detail(ui, "Nome:", file.name.clone());
                                        
                                        // Tamanho: para pastas, calcular conteúdo assíncrono
                                        let size_display = if file.is_dir {
                                            // Check if we have cached size
                                            if let Some(&cached_size) = self.folder_size_cache.get(&file.path) {
                                                format_size(cached_size)
                                            } else if self.folder_size_loading.contains(&file.path) {
                                                // Currently calculating
                                                "Calculando...".to_string()
                                            } else {
                                                // Trigger async calculation
                                                self.folder_size_loading.insert(file.path.clone());
                                                let _ = self.folder_size_req_sender.send(file.path.clone());
                                                "Calculando...".to_string()
                                            }
                                        } else {
                                            // Regular file - use file.size directly
                                            format_size(file.size)
                                        };
                                        add_detail(ui, "Tamanho:", size_display);

                                        let type_label = if file.is_dir {
                                            "Pasta".to_string()
                                        } else {
                                            file.path
                                                .extension()
                                                .and_then(|e: &std::ffi::OsStr| e.to_str())
                                                .unwrap_or("Arquivo")
                                                .to_uppercase()
                                        };
                                        add_detail(ui, "Tipo:", type_label);
                                        add_detail(ui, "Data:", format_date(file.modified));

                                        if let Some(meta) = selected_metadata {
                                            if let (Some(w), Some(h)) = (meta.width, meta.height) {
                                                add_detail(
                                                    ui,
                                                    "Resolução:",
                                                    format!("{} x {} px", w, h),
                                                );
                                            }

                                            if let Some(format) = &meta.format {
                                                add_detail(ui, "Formato:", format.clone());
                                            }

                                            if let Some(bits) = meta.color_depth {
                                                add_detail(
                                                    ui,
                                                    "Profundidade:",
                                                    format!("{} bits", bits),
                                                );
                                            }

                                            if let Some(maker) = &meta.camera_maker {
                                                add_detail(ui, "Fabricante:", maker.clone());
                                            }

                                            if let Some(model) = &meta.camera_model {
                                                add_detail(ui, "Modelo:", model.clone());
                                            }

                                            if let Some(date) = &meta.date_taken {
                                                add_detail(ui, "Captura:", date.clone());
                                            }

                                            if let Some(f_stop) = &meta.f_stop {
                                                add_detail(ui, "F-stop:", f_stop.clone());
                                            }

                                            if let Some(exposure) = &meta.exposure_time {
                                                add_detail(ui, "Exposição:", exposure.clone());
                                            }

                                            if let Some(iso) = meta.iso_speed {
                                                add_detail(ui, "ISO:", format!("ISO-{}", iso));
                                            }

                                            if let Some(focal) = &meta.focal_length {
                                                add_detail(ui, "Dist. Focal:", focal.clone());
                                            }

                                            if let Some(aperture) = &meta.max_aperture {
                                                add_detail(ui, "Abertura:", aperture.clone());
                                            }

                                            if let Some(metering) = &meta.metering_mode {
                                                add_detail(ui, "Medição:", metering.clone());
                                            }

                                            if let Some(flash) = &meta.flash_mode {
                                                add_detail(ui, "Flash:", flash.clone());
                                            }

                                            if let Some(subject) = &meta.subject {
                                                add_detail(ui, "Assunto:", subject.clone());
                                            }

                                            if let Some(codec) = &meta.video_codec {
                                                add_detail(ui, "Video Codec:", codec.clone());
                                            }

                                            if let Some(codec) = &meta.audio_codec {
                                                add_detail(ui, "Audio Codec:", codec.clone());
                                            }

                                            if let Some(bitrate) = meta.audio_bitrate {
                                                add_detail(
                                                    ui,
                                                    "Audio BR:",
                                                    Self::format_bitrate(bitrate),
                                                );
                                            }

                                            if let Some(channels) = meta.audio_channels {
                                                let channel_name = match channels {
                                                    1 => "Mono",
                                                    2 => "Estéreo",
                                                    6 => "5.1",
                                                    8 => "7.1",
                                                    _ => "Outro",
                                                };
                                                add_detail(
                                                    ui,
                                                    "Canais:",
                                                    format!("{} ({})", channels, channel_name),
                                                );
                                            }

                                            if let Some(duration) = meta.duration_100ns {
                                                add_detail(
                                                    ui,
                                                    "Duração:",
                                                    Self::format_media_duration(duration),
                                                );
                                            }

                                            if let Some(fps) = meta.frame_rate {
                                                add_detail(
                                                    ui,
                                                    "Frame rate:",
                                                    format!("{:.2} fps", fps),
                                                );
                                            }

                                            let mut bitrate_to_show = meta.bitrate;
                                            if bitrate_to_show.is_none() {
                                                if let Some(duration) = meta.duration_100ns {
                                                    bitrate_to_show = Self::approximate_bitrate(
                                                        file.size, duration,
                                                    );
                                                }
                                            }

                                            if let Some(bps) = bitrate_to_show {
                                                add_detail(
                                                    ui,
                                                    "Bitrate:",
                                                    Self::format_bitrate(bps),
                                                );
                                            }
                                        } else if is_loading_meta {
                                            add_detail(
                                                ui,
                                                "Metadados:",
                                                "Carregando...".to_string(),
                                            );
                                        }
                                    }
                                });
                            } else {
                                ui.vertical_centered(|ui| {
                                    ui.add_space(100.0);
                                    ui.label("Nenhum item selecionado");
                                    ui.label("Selecione algo para ver detalhes");
                                });
                            }
                        });
                });
            
            // Captura a largura REAL do painel direito
            // IMPORTANTE: Não atualiza se janela está minimizada (rect fica inválido)
            let is_minimized = ctx.input(|i| i.viewport().minimized.unwrap_or(false));
            let actual_panel_width = right_panel_response.response.rect.width();
            if !is_minimized && actual_panel_width > 200.0 && (self.sidebar_right_width - actual_panel_width).abs() > 2.0 {
                self.sidebar_right_width = actual_panel_width;
            }
        }

        // Central Panel
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.is_loading_folder && self.items.is_empty() {
                ui.centered_and_justified(|ui| {
                    ui.spinner();
                    ui.label("Carregando...");
                });
            } else if self.items.is_empty() {
                ui.centered_and_justified(|ui| {
                    ui.label("Pasta vazia");
                });
            } else {
                match self.view_mode {
                    ViewMode::Grid => self.render_grid_view(ui),
                    ViewMode::List => self.render_list_view(ui),
                }

                // F2 -> INICIAR RENOMEAÇÃO (Global no CentralPanel)
                if ui.input(|i| i.key_pressed(egui::Key::F2)) {
                    if let Some(idx) = self.selected_item {
                        if let Some(item) = self.items.get(idx) {
                            self.renaming_state = Some((idx, item.name.clone()));
                            self.focus_rename = true;
                        }
                    }
                }

                // Spinner pequeno no canto se ainda carregando
                if self.is_loading_folder {
                    let rect = ui.max_rect();
                    let spinner_rect = egui::Rect::from_min_size(
                        rect.right_bottom() - egui::vec2(24.0, 24.0),
                        egui::vec2(16.0, 16.0),
                    );
                    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(spinner_rect), |ui| {
                        ui.spinner();
                    });
                }
            }

            // Detecção de clique direito na área vazia (fora dos itens)
            // Só abre menu de contexto se não houver item selecionado pelo clique direito
            if !self.context_menu.is_open
                && ui.input(|i| i.pointer.secondary_clicked())
            {
                // Verifica se o clique foi em um item
                let pointer_pos = ui.ctx().pointer_latest_pos();
                let mut clicked_on_item = false;

                // Verifica se o clique foi em algum item (grid ou lista)
                if let Some(pos) = pointer_pos {
                    // Para grid view
                    if self.view_mode == ViewMode::Grid && !self.items.is_empty() {
                        let padding = 8.0;
                        let item_w = self.thumbnail_size;
                        let item_h = self.thumbnail_size + 20.0;
                        let available_w = ui.available_width();
                        let cols = ((available_w - padding) / (item_w + padding))
                            .floor()
                            .max(1.0) as usize;

                        // Calcula qual célula foi clicada
                        let content_min = ui.min_rect().min;
                        let relative_x = pos.x - content_min.x;
                        let relative_y = pos.y - content_min.y;

                        let col = (relative_x / (item_w + padding)).floor() as usize;
                        let row = (relative_y / (item_h + padding)).floor() as usize;
                        let index = row * cols + col;

                        if index < self.items.len() {
                            clicked_on_item = true;
                        }
                    }
                    // Para list view (mais simples - verifica se está na área dos itens)
                    else if self.view_mode == ViewMode::List && !self.items.is_empty() {
                        let row_height = 24.0;
                        let total_rows = self.items.len();
                        let scroll_area_top = ui.min_rect().top();
                        let relative_y = pos.y - scroll_area_top;

                        let row = (relative_y / row_height).floor() as usize;
                        if row < total_rows {
                            clicked_on_item = true;
                        }
                    }
                }

                // Se não clicou em item, abre menu de contexto estilizado para a pasta atual (área vazia)
                if !clicked_on_item {
                    let path = PathBuf::from(&self.current_path);
                    let pointer_pos = ui.ctx().pointer_latest_pos().unwrap_or(egui::Pos2::ZERO);
                    self.populate_context_menu(ui.ctx(), &path, true, None);
                    self.context_menu.open(
                        pointer_pos,
                        None,
                        Some(path),
                        true,
                    );
                }
            }
        });

        // Exibe o menu de contexto (se aberto)
        let mut context_menu = std::mem::replace(&mut self.context_menu, mtt_file_manager::application::context_menu::ContextMenuState::default());
        let _ = mtt_file_manager::ui::context_menu::render_context_menu(ctx, &mut context_menu, &mut self.svg_icon_manager);
        
        // Handle selected command before putting state back
        if let Some(id) = context_menu.selected_command_id.take() {
            if id > 0 {
                // Shell command
                if let Some(native_ctx) = &context_menu.native_context {
                    if let Some(shell_ctx) = native_ctx.downcast_ref::<mtt_file_manager::infrastructure::windows::native_menu::ShellMenuContext>() {
                        let _ = mtt_file_manager::infrastructure::windows::native_menu::invoke_menu_command(
                            self.native_hwnd.unwrap_or_default(),
                            &shell_ctx.context_menu,
                            id as u32,
                            context_menu.position.x as i32,
                            context_menu.position.y as i32,
                        );
                    }
                }
            } else {
                // Internal command handled via trait
                let item_idx = context_menu.item_index;
                eprintln!("[DEBUG] Internal command id: {}, item_idx: {:?}", id, item_idx);
                match id {
                    -1 => self.create_new_folder(),
                    -2 | -31 => self.command_copy(item_idx),
                    -3 | -30 => self.command_cut(item_idx),
                    -4 | -32 => self.command_paste(item_idx),
                    -5 | -33 => {
                        if let Some(idx) = item_idx.or(self.selected_item) {
                            if let Some(item) = self.items.get(idx) {
                                self.renaming_state = Some((idx, item.name.clone()));
                                self.focus_rename = true;
                            }
                        }
                    }
                    -6 | -34 => self.delete_with_shell_for_idx(item_idx),
                    -20 => {
                        // Abrir: Navigate into folder or open file with shell
                        if let Some(path) = self.context_target_path(item_idx) {
                            if path.is_dir() {
                                self.navigate_to(&path.to_string_lossy());
                            } else {
                                open_with_shell(&path);
                            }
                        }
                    }
                    -21 => {
                        if let Some(path) = self.context_target_path(item_idx) {
                            let target = if path.is_dir() {
                                path
                            } else {
                                path.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from(&self.current_path))
                            };

                            self.sync_to_tab();
                            self.tab_manager.new_tab_at(&target.to_string_lossy());
                            self.sync_from_tab();

                            if self.is_computer_view {
                                self.setup_computer_view();
                            } else {
                                self.watch_current_folder();
                                self.load_folder(false);
                            }
                        }
                    }
                    -24 => {
                        if let Some(path) = self.context_target_path(item_idx) {
                            self.copy_path_to_clipboard(&path);
                        }
                    }
                    -26 => {
                        if let Some(path) = self.context_target_path(item_idx) {
                            match self.create_shell_shortcut(&path) {
                                Ok(created) => {
                                    // Refresh to show the new shortcut in the view
                                    self.load_folder(false);
                                    self.notifications.push(
                                        mtt_file_manager::application::AppNotification::info(
                                            format!("Atalho criado: {}", created.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()),
                                        ),
                                    );
                                }
                                Err(e) => {
                                    self.notifications.push(
                                        mtt_file_manager::application::AppNotification::error(
                                            format!("Falha ao criar atalho: {e}"),
                                        ),
                                    );
                                }
                            }
                        }
                    }
                    -28 => self.show_properties_for_idx(item_idx),
                    // Recycle Bin actions
                    -50 | -52 => {
                        // Restaurar
                        if let Some(idx) = item_idx.or(self.selected_item) {
                            if let Some(item) = self.items.get(idx) {
                                let path = item.path.clone();
                                self.restore_from_recycle_bin(&path);
                            }
                        }
                    }
                    -51 | -53 => {
                        // Excluir permanentemente
                        if let Some(idx) = item_idx.or(self.selected_item) {
                            if let Some(item) = self.items.get(idx) {
                                let path = item.path.clone();
                                self.delete_permanently(&path);
                            }
                        }
                    }
                    -54 => {
                        // Esvaziar Lixeira
                        self.empty_recycle_bin();
                    }
                    _ => {}
                }
            }
            context_menu.close();
        }
        
        self.context_menu = context_menu;

        // === RESIZE GRIP (bottom-right corner) ===
        let is_not_maximized = !ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        if is_not_maximized {
            let screen_rect = ctx.screen_rect();
            
            // === BORDAS INVISÍVEIS PARA RESIZE (8px de largura) ===
            let border_width = 12.0;  // mais fácil de clicar
            
            // Borda ESQUERDA
            let left_border = egui::Rect::from_min_max(
                screen_rect.min,
                egui::pos2(screen_rect.min.x + border_width, screen_rect.max.y)
            );
            egui::Area::new(egui::Id::new("resize_border_left"))
                .fixed_pos(left_border.min)
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    let left_response = ui.interact(left_border, egui::Id::new("resize_left"), egui::Sense::click_and_drag());
                    if left_response.hovered() {
                        ctx.set_cursor_icon(egui::CursorIcon::ResizeWest);
                    }
                    if left_response.drag_started() {
                        // Usa egui BeginResize - funciona mas tem efeito sanfona no lado esquerdo
                        ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(egui::ResizeDirection::West));
                    }
                });
            
            // Borda DIREITA
            let right_border = egui::Rect::from_min_max(
                egui::pos2(screen_rect.max.x - border_width, screen_rect.min.y),
                screen_rect.max
            );
            egui::Area::new(egui::Id::new("resize_border_right"))
                .fixed_pos(right_border.min)
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    let right_response = ui.interact(right_border, egui::Id::new("resize_right"), egui::Sense::click_and_drag());
                    if right_response.hovered() {
                        ctx.set_cursor_icon(egui::CursorIcon::ResizeEast);
                    }
                    if right_response.drag_started() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(egui::ResizeDirection::East));
                    }
                });
            
            // Borda INFERIOR
            let bottom_border = egui::Rect::from_min_max(
                egui::pos2(screen_rect.min.x, screen_rect.max.y - border_width),
                screen_rect.max
            );
            egui::Area::new(egui::Id::new("resize_border_bottom"))
                .fixed_pos(bottom_border.min)
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    let bottom_response = ui.interact(bottom_border, egui::Id::new("resize_bottom"), egui::Sense::click_and_drag());
                    if bottom_response.hovered() {
                        ctx.set_cursor_icon(egui::CursorIcon::ResizeSouth);
                    }
                    if bottom_response.drag_started() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(egui::ResizeDirection::South));
                    }
                });
            
            // === GRIP VISUAL (canto inferior direito - 50x50px) ===
            let grip_size = 50.0;  // MUITO maior para ser facilmente clicável
            let grip_pos = egui::pos2(
                screen_rect.max.x - grip_size,
                screen_rect.max.y - grip_size,
            );
            let grip_rect = egui::Rect::from_min_size(grip_pos, egui::vec2(grip_size, grip_size));
            
            egui::Area::new(egui::Id::new("resize_grip"))
                .fixed_pos(grip_pos)
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    let (_rect, response) = ui.allocate_exact_size(
                        egui::vec2(grip_size, grip_size),
                        egui::Sense::click_and_drag(),
                    );
                    
                    // SEM VISUAL - apenas área interativa (sem listras aparecendo por cima)
                    
                    // Handle resize drag - só dispara no início do drag
                    if response.drag_started() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(egui::ResizeDirection::SouthEast));
                    }
                    
                    // Change cursor on hover
                    if response.hovered() {
                        ctx.set_cursor_icon(egui::CursorIcon::ResizeNwSe);
                    }
                });
        }
        
        // === TOAST NOTIFICATIONS ===
        self.notifications.cleanup(); // Remove expired notifications

        if !self.notifications.is_empty() {
            let toast_width = 300.0;
            let toast_height = 40.0;
            let padding = 10.0;
            let margin = 20.0;

            let screen = ctx.screen_rect();
            let base_x = screen.max.x - toast_width - margin;

            for (i, notification) in self.notifications.active().iter().enumerate() {
                let base_y = screen.max.y - margin - ((i + 1) as f32 * (toast_height + padding));
                let fade = notification.remaining_fraction();

                let mut bg_color = notification.level.color();
                bg_color = egui::Color32::from_rgba_unmultiplied(
                    bg_color.r(),
                    bg_color.g(),
                    bg_color.b(),
                    (fade * 230.0) as u8,
                );

                egui::Area::new(egui::Id::new(format!("toast_{}", i)))
                    .fixed_pos(egui::pos2(base_x, base_y))
                    .order(egui::Order::Foreground)
                    .show(ctx, |ui| {
                        let rect = egui::Rect::from_min_size(
                            ui.cursor().min,
                            egui::vec2(toast_width, toast_height),
                        );

                        ui.painter().rect_filled(rect, 6.0, bg_color);

                        // Icon
                        ui.painter().text(
                            rect.min + egui::vec2(12.0, 12.0),
                            egui::Align2::LEFT_TOP,
                            notification.level.icon(),
                            egui::FontId::proportional(14.0),
                            egui::Color32::WHITE.gamma_multiply(fade),
                        );

                        // Message
                        ui.painter().text(
                            rect.min + egui::vec2(32.0, 12.0),
                            egui::Align2::LEFT_TOP,
                            &notification.message,
                            egui::FontId::proportional(13.0),
                            egui::Color32::WHITE.gamma_multiply(fade),
                        );
                    });
            }
            ctx.request_repaint(); // Keep animating
        }

    }

    /// Called when the app is exiting - save all preferences
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Force save sidebar widths before exit
        self.save_preferences();
        eprintln!("[EXIT] Saved sidebar widths: L={}, R={}", self.sidebar_left_width, self.sidebar_right_width);
    }
}

/// Load application icon from PNG file
fn load_app_icon() -> Option<egui::IconData> {
    let icon_path = std::path::PathBuf::from("appicon.png");
    
    if !icon_path.exists() {
        eprintln!("Warning: appicon.png not found - using default icon");
        return None;
    }
    
    // Load PNG using image crate
    match image::open(&icon_path) {
        Ok(img) => {
            // Resize to 256x256 for optimal display (Windows icon standard)
            let resized = img.resize_exact(256, 256, image::imageops::FilterType::Lanczos3);
            let rgba_image = resized.to_rgba8();
            let pixels = rgba_image.into_raw();
            
            Some(egui::IconData {
                rgba: pixels,
                width: 256,
                height: 256,
            })
        }
        Err(e) => {
            eprintln!("Warning: Failed to load appicon.png: {}", e);
            None
        }
    }
}

fn main() -> eframe::Result<()> {
    // Initialize codec name cache (queries Windows Registry on-demand)
    mtt_file_manager::infrastructure::windows::codec_registry::init_codec_cache();
    
    // Load application icon
    let icon_data = load_app_icon();
    
    // 3-STAGE STARTUP: Start hidden and small (NOT maximized here)
    let mut viewport = egui::ViewportBuilder::default()
        .with_visible(false) // Start hidden
        .with_maximized(false) // NOT maximized at creation
        .with_inner_size([800.0, 600.0]) // Small initial size (will be maximized in update)
        .with_title("MTT File Manager")
        .with_app_id("mtt-file-manager")
        .with_decorations(true) // Use native Windows title bar (fixes resize and sidebar issues)
        .with_resizable(true); // HABILITA resize nativo do Windows
    
    // Set window icon if loaded successfully
    if let Some(icon) = icon_data {
        viewport = viewport.with_icon(icon);
    }
    
    let options = eframe::NativeOptions {
        viewport,
        persist_window: false, // Disable eframe persistence - we control manually
        ..Default::default()
    };

    eframe::run_native(
        "MTT File Manager",
        options,
        Box::new(|cc| {
            // Carrega Segoe UI (fonte do Windows Explorer) + Symbol para Unicode completo
            let mut fonts = egui::FontDefinitions::default();
            let mut loaded_fonts = Vec::new();

            // 1. Segoe UI (fonte principal)
            let segoe_path = std::path::PathBuf::from("C:\\Windows\\Fonts\\segoeui.ttf");
            if let Ok(font_data) = std::fs::read(&segoe_path) {
                fonts.font_data.insert(
                    "segoe_ui".to_owned(),
                    std::sync::Arc::new(egui::FontData::from_owned(font_data)),
                );
                loaded_fonts.push("segoe_ui".to_owned());
            }

            // 2. Segoe UI Symbol (fallback 1 - símbolos)
            let symbol_path = std::path::PathBuf::from("C:\\Windows\\Fonts\\seguisym.ttf");
            if let Ok(font_data) = std::fs::read(&symbol_path) {
                fonts.font_data.insert(
                    "segoe_ui_symbol".to_owned(),
                    std::sync::Arc::new(egui::FontData::from_owned(font_data)),
                );
                loaded_fonts.push("segoe_ui_symbol".to_owned());
            }

            // 3. Arial Unicode MS (fallback 2 - se disponível)
            let arial_path = std::path::PathBuf::from("C:\\Windows\\Fonts\\ARIALUNI.TTF");
            if let Ok(font_data) = std::fs::read(&arial_path) {
                fonts.font_data.insert(
                    "arial_unicode".to_owned(),
                    std::sync::Arc::new(egui::FontData::from_owned(font_data)),
                );
                loaded_fonts.push("arial_unicode".to_owned());
            }

            // 4. Remix Icon (Fonte de Ícones dedicada)
            if let Ok(data) = std::fs::read("assets/remixicon.ttf") {
                fonts.font_data.insert(
                    "remix_icon".to_owned(),
                    std::sync::Arc::new(egui::FontData::from_owned(data)),
                );

                // Definir uma família específica para ícones
                fonts.families.insert(
                    egui::FontFamily::Name("icons".into()),
                    vec!["remix_icon".to_owned()],
                );
            }

            // Adiciona apenas fontes carregadas
            if !loaded_fonts.is_empty() {
                fonts
                    .families
                    .get_mut(&egui::FontFamily::Proportional)
                    .unwrap()
                    .extend(loaded_fonts.clone());

                fonts
                    .families
                    .get_mut(&egui::FontFamily::Monospace)
                    .unwrap()
                    .extend(loaded_fonts.clone());
            }

            cc.egui_ctx.set_fonts(fonts);

            Ok(Box::new(ImageViewerApp::new(cc)))
        }),
    )
}
