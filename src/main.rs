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
use mtt_file_manager::ui::context_menu::render_context_menu;
use mtt_file_manager::ui::icon_loader::IconLoader;

use windows::{
    core::*,
    Win32::Foundation::*,
    Win32::Storage::FileSystem::*,
    Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState,
    Win32::UI::Shell::*,
    Win32::UI::WindowsAndMessaging::{FindWindowW, GetCursorPos},
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
    selected_metadata: Option<(PathBuf, windows_infra::MediaMetadata)>,
    metadata_req_sender: Sender<(PathBuf, u64)>,
    metadata_res_receiver: Receiver<(PathBuf, u64, windows_infra::MediaMetadata)>,
    metadata_cache: LruCache<PathBuf, (u64, windows_infra::MediaMetadata)>,
    metadata_loading: HashSet<PathBuf>,
    show_preview_panel: bool,
    is_computer_view: bool, // Se estamos na view "Este Computador"

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
    onedrive_icon: Option<egui::TextureHandle>, // Ícone nativo do OneDrive

    // NAVEGAÇÃO / ADDRESS BAR (Breadcrumbs vs Edit)
    is_address_editing: bool,

    // SCROLL TO SELECTED (para navegação por teclado)
    scroll_to_selected: bool,

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
            selected_metadata: None,
            show_preview_panel, // Loaded from SQLite
            is_computer_view: false,
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

            // METADATA ASYNC
            metadata_req_sender: meta_req_tx,
            metadata_res_receiver: meta_res_rx,
            metadata_cache: LruCache::new(NonZeroUsize::new(512).unwrap()),
            metadata_loading: HashSet::new(),
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

        app.load_folder(false);
        app
    }
}

impl ImageViewerApp {
    // Helper para botÃµes de Ã­cone da Toolbar
    fn icon_button(&self, ui: &mut egui::Ui, icon: &str, tooltip: &str) -> egui::Response {
        let rich_text = egui::RichText::new(icon)
            .family(egui::FontFamily::Name("icons".into()))
            .size(22.0);

        let btn = egui::Button::new(rich_text).frame(false);
        ui.add(btn).on_hover_text(tooltip)
    }

    fn delete_with_shell(&mut self) {
        if let Some(idx) = self.selected_item {
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
                        self.selected_item = None;
                        self.selected_file = None;
                    }
                }
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
    fn command_copy(&mut self) {
        if let Some(idx) = self.selected_item {
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
    fn command_cut(&mut self) {
        if let Some(idx) = self.selected_item {
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
    fn command_paste(&mut self) {
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

        // 2. Destination folder (current directory)
        let dest_folder = PathBuf::from(&self.current_path);

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
        &self,
        ui: &mut egui::Ui,
        icon: &str,
        active: bool,
        tooltip: &str,
    ) -> egui::Response {
        let color = if active {
            egui::Color32::from_rgb(0, 120, 215)
        } else {
            ui.visuals().text_color()
        };

        let rich_text = egui::RichText::new(icon)
            .family(egui::FontFamily::Name("icons".into()))
            .size(22.0)
            .color(color);

        // Removemos o .fill(bg) para retirar o "glow" azul
        let btn = egui::Button::new(rich_text).frame(false);
        ui.add(btn).on_hover_text(tooltip)
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

                            // Get extended attributes using GetFileAttributesEx for OneDrive cloud file attributes
                            let extended_attrs = if is_onedrive {
                                let path_wide: Vec<u16> = full_path
                                    .to_string_lossy()
                                    .encode_utf16()
                                    .chain(std::iter::once(0))
                                    .collect();

                                use windows::Win32::Storage::FileSystem::{
                                    GetFileAttributesW, INVALID_FILE_ATTRIBUTES,
                                };
                                match unsafe {
                                    GetFileAttributesW(windows::core::PCWSTR(path_wide.as_ptr()))
                                } {
                                    result if result != INVALID_FILE_ATTRIBUTES => result,
                                    _ => attrs, // Fallback to basic attributes
                                }
                            } else {
                                attrs
                            };

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
                                let mut sync_status =
                                    onedrive::get_sync_status(extended_attrs, is_onedrive);

                                // If file is open in an application, mark as syncing
                                // (this mimics Windows Explorer behavior showing syncing icon for open files)
                                if is_onedrive && !is_dir && sync_status != SyncStatus::None {
                                    if onedrive::is_file_open(&full_path) {
                                        sync_status = SyncStatus::Syncing;
                                    }
                                }

                                let entry = FileEntry {
                                    path: full_path,
                                    name: filename,
                                    is_dir,
                                    size,
                                    modified,
                                    folder_cover,
                                    drive_info: None,
                                    sync_status,
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
        self.path_input = normalized_path;
        self.is_computer_view = false;

        // Limpa o context_menu.target_path para garantir sincronia com a pasta atual
        self.context_menu.target_path = None;

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
                self.setup_computer_view();
            } else {
                let new_path = std::path::PathBuf::from(&path);
                
                // Se estávamos em uma subpasta do destino, invalida o preview dessa subpasta
                if previous_path.starts_with(&new_path) && previous_path != new_path {
                    self.cache_manager.invalidate_folder_preview(&previous_path);
                }
                
                self.current_path = path;
                self.path_input = self.current_path.clone();
                self.is_computer_view = false;
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
                self.setup_computer_view();
            } else {
                let new_path = std::path::PathBuf::from(&path);
                
                // Se o destino é pai do path atual, invalida o preview do path atual
                if previous_path.starts_with(&new_path) && previous_path != new_path {
                    self.cache_manager.invalidate_folder_preview(&previous_path);
                }
                
                self.current_path = path;
                self.path_input = self.current_path.clone();
                self.is_computer_view = false;
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

        // Corta histórico "futuro"
        if self.history_index < self.navigation_history.len().saturating_sub(1) {
            self.navigation_history.truncate(self.history_index + 1);
        }

        // Adiciona ao histórico
        self.navigation_history.push("Este Computador".to_string());
        self.history_index = self.navigation_history.len() - 1;

        let _ = self.reload_drive_list();
        self.last_drive_refresh = Instant::now();
        self.setup_computer_view();
    }

    /// Configura a visão de "Este Computador" sem afetar o histórico
    fn setup_computer_view(&mut self) {
        // Set computer view
        self.current_path = "Este Computador".to_string();
        self.is_computer_view = true;
        self.path_input = "Este Computador".to_string();

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
        self.selected_item = None;
        self.selected_file = None;
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
            }
        }
    }

    /// Agenda abertura do menu de contexto nativo para após a UI ser renderizada.
    /// Isso permite que a UI repinte a seleção visual antes do menu aparecer.
    /// Retorna true em caso de tentativa.
    fn try_show_shell_context_menu(&mut self, ui: &egui::Ui, path: &Path) -> bool {
        if self.native_hwnd.is_some() {
            let mut cursor = POINT::default();
            unsafe {
                let _ = GetCursorPos(&mut cursor);
            }
            // Store pending menu request with 1 frame delay
            // This ensures the UI is fully rendered before the menu appears
            self.context_menu.pending_native_menu =
                Some((path.to_path_buf(), cursor.x, cursor.y, 1));
            // Request immediate repaint so the selection is visible
            ui.ctx().request_repaint();
            true
        } else {
            false
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

        // Para executaveis, usa path real; para demais, usa extensao dummy
        let icon_result = if matches!(extension.as_str(), "exe" | "lnk" | "ico") {
            extract_file_icon_by_path(path, icon_size)
        } else {
            extract_file_icon(&format!(".{}", extension), icon_size)
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
                    .put_thumbnail(thumbnail_data.path, texture);
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


        if received_any {
            ctx.request_repaint();
        }
    }

    // --- DETALHES (LIST VIEW) ---
    fn render_list_view(&mut self, ui: &mut egui::Ui) {
        use mtt_file_manager::ui::views::{list_view, ListViewContext, ListViewOperations};

        // Keyboard navigation for list view (ONLY when not renaming)
        if self.renaming_state.is_none() {
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
                    self.selected_file = Some(item.clone());
                    self.selected_item = Some(clamped);
                    self.scroll_to_selected = true; // Trigger scroll to selected item

                    // Trigger thumbnail load for sidebar preview
                    if !item.is_dir {
                        if !self.cache_manager.has_thumbnail(&item.path)
                            && !self.cache_manager.is_loading(&item.path)
                        {
                            self.request_thumbnail_load(item.path.clone());
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
            is_onedrive_folder,
            texture_cache: &mut self.cache_manager.texture_cache,
            loading_set: &mut self.cache_manager.loading_set,
            scanned_folders: &mut self.scanned_folders,
            folder_icon_texture: folder_icon_texture.as_ref(),
            computer_icon: computer_icon.as_ref(),
            drive_icon_cache: &mut self.cache_manager.drive_icon_cache,
            item_icon_loader: &mut self.item_icon_loader,
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
                    self.selected_file = Some(item.clone());

                    // Trigger thumbnail load for sidebar preview
                    if !item.is_dir {
                        if !self.cache_manager.has_thumbnail(&item.path)
                            && !self.cache_manager.is_loading(&item.path)
                        {
                            self.request_thumbnail_load(item.path.clone());
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

                    // Step 2: Store pending menu data and mark that we need to draw first
                    if self.native_hwnd.is_some() {
                        let mut cursor = POINT::default();
                        unsafe {
                            let _ = GetCursorPos(&mut cursor);
                        }
                        // Menu will open after the selection is drawn (needs_draw_before_menu flag)
                        self.context_menu.pending_native_menu =
                            Some((item_path.clone(), cursor.x, cursor.y, 0));
                        self.context_menu.needs_draw_before_menu = true;
                        // Request repaint to ensure selection is drawn before menu
                        ui.ctx().request_repaint();
                    } else {
                        // Fallback: use egui context menu
                        self.context_menu.open(
                            ui.ctx().pointer_latest_pos().unwrap_or(egui::Pos2::ZERO),
                            Some(idx),
                            Some(item_path),
                            false,
                        );
                    }
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
        if self.renaming_state.is_none() {
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
                    self.scroll_to_selected = true; // Trigger scroll to selected item
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

                    // Step 2: Store pending menu data and mark that we need to draw first
                    if self.native_hwnd.is_some() {
                        let mut cursor = POINT::default();
                        unsafe {
                            let _ = GetCursorPos(&mut cursor);
                        }
                        // Menu will open after the selection is drawn (needs_draw_before_menu flag)
                        self.context_menu.pending_native_menu =
                            Some((item_path.clone(), cursor.x, cursor.y, 0));
                        self.context_menu.needs_draw_before_menu = true;
                        // Request repaint to ensure selection is drawn before menu
                        ui.ctx().request_repaint();
                    } else {
                        // Fallback: use egui context menu
                        self.context_menu.open(
                            ui.ctx().pointer_latest_pos().unwrap_or(egui::Pos2::ZERO),
                            Some(idx),
                            Some(item_path),
                            false,
                        );
                    }
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

    fn command_copy(&mut self) {
        self.command_copy();
    }

    fn command_cut(&mut self) {
        self.command_cut();
    }

    fn command_paste(&mut self) {
        self.command_paste();
    }

    fn rename_item(&mut self, idx: usize) {
        if let Some(item) = self.items.get(idx) {
            self.renaming_state = Some((idx, item.name.clone()));
            self.focus_rename = true;
        }
    }

    fn delete_with_shell(&mut self) {
        self.delete_with_shell();
    }
}

impl eframe::App for ImageViewerApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
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
            }
            
            // Keep the loop running fast during startup
            ctx.request_repaint();
        }
        // --- END STARTUP SEQUENCE ---

        // Track current window state for saving on exit
        ctx.input(|i| {
            if let Some(rect) = i.viewport().inner_rect {
                // Only save size when NOT maximized
                if !i.viewport().maximized.unwrap_or(false) {
                    self.saved_window_width = rect.width();
                    self.saved_window_height = rect.height();
                }
            }
            self.saved_is_maximized = i.viewport().maximized.unwrap_or(false);
        });
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
                self.command_copy();
            }
            if do_cut {
                self.command_cut();
            }
            if do_paste {
                self.command_paste();
            }

            // Delete: Excluir
            if ctx.input(|i| i.key_pressed(egui::Key::Delete)) {
                self.delete_with_shell();
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

        // Top navigation bar
        egui::TopBottomPanel::top("nav_bar").show(ctx, |ui| {
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

                // Botão de Nova Pasta mais visível (agora sem fundo para combinar)
                let btn_text = egui::RichText::new(format!("+ {}", ICON_FOLDER_ADD))
                    .family(egui::FontFamily::Name("icons".into()))
                    .size(22.0);

                let btn = egui::Button::new(btn_text).frame(false);
                if ui
                    .add(btn)
                    .on_hover_text("Criar Nova Pasta (Ctrl+Shift+N)")
                    .clicked()
                    && !is_renaming
                {
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
                    ui.label(
                        egui::RichText::new(ICON_SEARCH)
                            .family(egui::FontFamily::Name("icons".into()))
                            .size(16.0),
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
        let sidebar_action = egui::SidePanel::left("sidebar")
            .min_width(200.0)
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
                    computer_icon: computer_icon.as_ref(),
                    is_renaming: self.renaming_state.is_some(),
                    icon_loader: &mut self.item_icon_loader,
                    onedrive_path: self.onedrive_path.as_deref(),
                    onedrive_icon: self.onedrive_icon.as_ref(),
                };

                render_sidebar(ui, &mut ctx)
            })
            .inner;

        // Processar ação da sidebar (após ctx ser dropado e self liberado)
        if let Some(action) = sidebar_action {
            use mtt_file_manager::ui::sidebar::SidebarAction;
            match action {
                SidebarAction::NavigateTo(path) => self.navigate_to(&path),
                SidebarAction::NavigateToComputer => self.navigate_to_computer(),
            }
        }

        // Preview Pane (Windows Explorer style) - ANTES do CentralPanel
        if self.show_preview_panel {
            self.refresh_selected_metadata();
            egui::SidePanel::right("preview_panel")
                .resizable(true)
                .default_width(300.0)
                .min_width(250.0)
                .max_width(500.0)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .id_source("preview_scroll")
                        .show(ui, |ui| {
                            ui.set_max_width(ui.available_width());
                            if let Some(file) = self.selected_file.clone() {
                                ui.heading("Detalhes");
                                ui.separator();

                                // Preview de imagem/video (se houver thumbnail)
                                let _has_thumbnail =
                                    self.cache_manager.texture_cache.peek(&file.path).is_some();
                                // Detecta se é mídia usando Windows Perceived Type API
                                let is_media = file
                            .path
                            .extension()
                            .map(|ext| {
                                mtt_file_manager::infrastructure::windows::is_media_extension(
                                    &ext.to_string_lossy(),
                                )
                            })
                            .unwrap_or(false);

                                let texture =
                                    self.cache_manager.texture_cache.peek(&file.path).cloned();

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
                                    
                                    // Botão de recarregar thumbnail
                                    ui.horizontal(|ui| {
                                        ui.add_space(ui.available_width() / 2.0 - 50.0);
                                        if ui.button("🔄 Recarregar").on_hover_text("Força re-extração do thumbnail (bypassa cache do Windows)").clicked() {
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
                                        } else if file.is_dir {
                                            // PASTA (Usa preview nativo do Windows - sandwich effect)
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
                                        } else {
                                            // ARQUIVO SEM THUMBNAIL
                                            if let Some(icon) =
                                                self.get_or_load_icon(ui.ctx(), &file.path)
                                            {
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
                                        add_detail(ui, "Tamanho:", format_size(file.size));

                                        let type_label = if file.is_dir {
                                            "Pasta".to_string()
                                        } else {
                                            file.path
                                                .extension()
                                                .and_then(|e| e.to_str())
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
                                    ui.label("Selecione um arquivo");
                                    ui.label("ou drive para ver detalhes");
                                });
                            }
                        });
                });
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
            // e não houver menu pendente (já definido pelo grid_view ou list_view)
            if !self.context_menu.is_open
                && self.context_menu.pending_native_menu.is_none()
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

                // Se não clicou em item, abre menu de contexto nativo para a pasta atual (área vazia)
                if !clicked_on_item {
                    // Use native Windows context menu for the current folder
                    if self.native_hwnd.is_some() && !self.is_computer_view {
                        let mut cursor = POINT::default();
                        unsafe {
                            let _ = GetCursorPos(&mut cursor);
                        }
                        // Store pending menu for the current folder (not a specific item)
                        self.context_menu.pending_native_menu =
                            Some((PathBuf::from(&self.current_path), cursor.x, cursor.y, 0));
                        ui.ctx().request_repaint();
                    } else {
                        // Fallback to egui context menu
                        self.context_menu.open(
                            pointer_pos.unwrap_or(
                                ui.ctx().pointer_latest_pos().unwrap_or(egui::Pos2::ZERO),
                            ),
                            None,
                            Some(PathBuf::from(&self.current_path)),
                            true,
                        );
                    }
                }
            }
        });

        // Exibe o menu de contexto (se aberto)
        let mut context_menu = self.context_menu.clone();
        let clipboard_file = self.clipboard_file.clone();
        render_context_menu(ctx, &mut context_menu, &clipboard_file, self);
        self.context_menu = context_menu;

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

        // --- PENDING NATIVE CONTEXT MENU ---
        // Open the context menu after a delay that allows the GPU to render the selection
        // We use request_repaint_after to schedule a repaint, then check if enough time has passed
        if let Some((path, screen_x, screen_y, start_time_ms)) =
            self.context_menu.pending_native_menu.take()
        {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;

            if start_time_ms == 0 {
                // First frame after right-click - record the start time and request repaint after delay
                let start = now;
                self.context_menu.pending_native_menu = Some((path, screen_x, screen_y, start));
                ctx.request_repaint_after(std::time::Duration::from_millis(60));
            } else if now - start_time_ms >= 50 {
                // Enough time has passed - GPU should have rendered by now, open the menu
                if let Some(hwnd) = self.native_hwnd {
                    match windows_infra::show_shell_context_menu(hwnd, &path, screen_x, screen_y) {
                        Ok(result) => {
                            if result.was_cancelled {
                                // Menu was cancelled by clicking outside - store the click for replay
                                // Check if the click position is different from the menu position (user clicked somewhere else)
                                let click_moved = (result.cursor_x - screen_x).abs() > 5
                                    || (result.cursor_y - screen_y).abs() > 5;
                                if click_moved {
                                    self.context_menu.pending_click_replay = Some((
                                        result.cursor_x,
                                        result.cursor_y,
                                        result.right_button_down,
                                    ));
                                    ctx.request_repaint();
                                }
                            }
                        }
                        Err(err) => {
                            eprintln!("Falha ao abrir menu de contexto do Windows: {:?}", err);
                        }
                    }
                }
            } else {
                // Not enough time yet - keep waiting
                self.context_menu.pending_native_menu =
                    Some((path, screen_x, screen_y, start_time_ms));
                ctx.request_repaint_after(std::time::Duration::from_millis(10));
            }
        }
        self.context_menu.needs_draw_before_menu = false;

        // --- REPLAY PENDING CLICK ---
        // If a click was consumed by context menu dismissal, replay it using SendInput
        if let Some((click_x, click_y, is_right_click)) =
            self.context_menu.pending_click_replay.take()
        {
            use windows::Win32::UI::Input::KeyboardAndMouse::*;
            use windows::Win32::UI::WindowsAndMessaging::{
                GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN,
            };

            unsafe {
                // Move mouse to the click position and simulate click
                let screen_width = GetSystemMetrics(SM_CXSCREEN);
                let screen_height = GetSystemMetrics(SM_CYSCREEN);

                // Normalize coordinates for absolute mouse position (0-65535)
                let norm_x = (click_x * 65535) / screen_width;
                let norm_y = (click_y * 65535) / screen_height;

                let button_down = if is_right_click {
                    MOUSEEVENTF_RIGHTDOWN
                } else {
                    MOUSEEVENTF_LEFTDOWN
                };
                let button_up = if is_right_click {
                    MOUSEEVENTF_RIGHTUP
                } else {
                    MOUSEEVENTF_LEFTUP
                };

                let inputs = [
                    INPUT {
                        r#type: INPUT_MOUSE,
                        Anonymous: INPUT_0 {
                            mi: MOUSEINPUT {
                                dx: norm_x,
                                dy: norm_y,
                                mouseData: 0,
                                dwFlags: MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_MOVE | button_down,
                                time: 0,
                                dwExtraInfo: 0,
                            },
                        },
                    },
                    INPUT {
                        r#type: INPUT_MOUSE,
                        Anonymous: INPUT_0 {
                            mi: MOUSEINPUT {
                                dx: norm_x,
                                dy: norm_y,
                                mouseData: 0,
                                dwFlags: MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_MOVE | button_up,
                                time: 0,
                                dwExtraInfo: 0,
                            },
                        },
                    },
                ];

                SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
            }
        }
    }

    /// Called when the app is exiting - save all preferences
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_preferences();
    }
}
fn main() -> eframe::Result<()> {
    // 3-STAGE STARTUP: Start hidden and small (NOT maximized here)
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_visible(false) // Start hidden
            .with_maximized(false) // NOT maximized at creation
            .with_inner_size([800.0, 600.0]) // Small initial size (will be maximized in update)
            .with_title("MTT File Manager")
            .with_app_id("mtt-file-manager"),
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
