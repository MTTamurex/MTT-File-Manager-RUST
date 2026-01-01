use eframe::egui;
use lru::LruCache;
use std::cmp::Ordering;
use std::collections::HashSet;
// use std::env;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};
use notify::{Watcher, RecursiveMode, RecommendedWatcher};

// Mapeamento Remix Icon
const ICON_ARROW_LEFT: &str = "\u{EA64}";  // Seta Esq
const ICON_ARROW_RIGHT: &str = "\u{EA6E}"; // Seta Dir
const ICON_ARROW_UP: &str = "\u{EA78}";    // Seta Cima
const ICON_REFRESH: &str = "\u{F064}";     // Recarregar
const ICON_HOME: &str = "\u{EE1B}";        // Casa/PC
const ICON_GRID: &str = "\u{ED9E}";        // Grade (Nova sugestão)
const ICON_LIST: &str = "\u{EF3E}";        // Lista
const ICON_SEARCH: &str = "\u{F0D1}";      // Lupa
const ICON_FOLDER_ADD: &str = "\u{ED5A}";  // Nova Pasta (Sugestão do usuário)
const ICON_DETAILS: &str = "\u{ECEA}";     // Detalhes (file-info-line)
const ICON_FOLDER: &str = "\u{ED9F}";      // Folder (folder-line)
const ICON_FILE: &str = "\u{ECD3}";        // File (file-line)


// Import domain types
use mtt_file_manager::application::context_menu::ContextMenuState;
use mtt_file_manager::domain::file_entry::*;
use mtt_file_manager::domain::thumbnail::*;

// Import infrastructure modules
use mtt_file_manager::infrastructure::windows as windows_infra;

// Import UI modules
// use mtt_file_manager::ui::status_bar; // Not used directly - imported in render_status_bar call
use mtt_file_manager::ui::context_menu::{render_context_menu, ContextMenuOperations};
use mtt_file_manager::ui::icon_loader::IconLoader;

use windows::{
    core::*,
    Win32::Foundation::*,
    Win32::Storage::FileSystem::*,
    Win32::UI::Shell::*,
};

// OTIMIZAÃ‡ÃƒO: Imports para Win32 FindFirst/NextFileW (metadata em UMA syscall)
use windows::Win32::Storage::FileSystem::{
    FindFirstFileW, FindNextFileW, FindClose, WIN32_FIND_DATAW, FILE_ATTRIBUTE_DIRECTORY
};
use std::os::windows::ffi::OsStringExt;

// Import specific Windows API functions from modules
use windows_infra::{
    get_all_drives,
    extract_file_icon,
    extract_file_icon_by_path,
    extract_drive_icon,
    open_with_shell,
    format_size,
    format_date,
};




// Caminho padrão
const PATH_PADRAO: &str = "C:\\";

// LRU cache - limita VRAM (~50-100MB)
const CACHE_SIZE: usize = 200;

// Icon cache (menor pois ícones são compartilhados por extensão)
const ICON_CACHE_SIZE: usize = 100;

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
    image_receiver: Receiver<ThumbnailData>,       // Worker Pool -> UI
    
    // File system
    items: Arc<Vec<FileEntry>>,  // Arc para clone barato em render loops (60 FPS)
    texture_cache: LruCache<PathBuf, egui::TextureHandle>,
    loading_set: HashSet<PathBuf>,
    
    // Async loading (evita freeze da UI ao ler metadata)
    file_entry_receiver: Receiver<(usize, Vec<FileEntry>)>,
    file_entry_sender: Sender<(usize, Vec<FileEntry>)>,
    is_loading_folder: bool,
    
    // COVER WORKER: Sistema de capas de pasta (Single Thread Worker)
    cover_worker_sender: Sender<PathBuf>,  // UI â†’ Worker: Envia pasta para processar
    cover_worker_receiver: Receiver<(PathBuf, Option<PathBuf>)>,  // Worker â†’ UI: Resultado
    scanned_folders: HashSet<PathBuf>,  // Cache: evita re-scan
    
    // Icon cache (novo: extensÃ£o â†’ texture)
    icon_cache: LruCache<String, egui::TextureHandle>,
    folder_icon_texture: Option<egui::TextureHandle>,
    computer_icon: Option<egui::TextureHandle>,  // Ãcone "Este Computador"
    drive_icon_cache: LruCache<String, egui::TextureHandle>,  // path â†’ icon
    
    // Sorting state
    sort_mode: SortMode,
    sort_descending: bool,  // true = Z-A, Mais Novo, Maior
    
    // View Mode
    view_mode: ViewMode,
    
    // Navigation state (histÃ³rico linear)
    navigation_history: Vec<String>,  // HistÃ³rico completo de paths
    history_index: usize,             // PosiÃ§Ã£o atual no histÃ³rico
    path_input: String,               // Barra de endereÃ§o editÃ¡vel
    
    // UI state
    disks: Vec<(String, String)>,  // (path, label)
    thumbnail_size: f32,        // Zoom: 64-512
    selected_item: Option<usize>,
    selected_file: Option<FileEntry>,
    show_preview_panel: bool,
    is_computer_view: bool,     // Se estamos na view "Este Computador"
    
    total_items: usize,
    
    // Search & Navigation (NEW)
    all_items: Vec<FileEntry>,  // Cache mestre para busca
    search_query: String,       // Texto da busca
    last_grid_cols: usize,      // Memória para navegação vertical (teclado)
    generation: usize,          // Contador local (Main Thread)
    current_generation: Arc<AtomicUsize>, // Contador compartilhado (Workers)
    ui_ctx: egui::Context,      // Referência ao contexto da UI para repaints assíncronos
    
    // ESTADO DE RENOMEAÇÃO
    renaming_state: Option<(usize, String)>, // (Index, Texto Editável)
    focus_rename: bool,                      // Trigger para focar no input
    
    // SISTEMA DE WATCHER (AUTO-REFRESH)
    watcher: Option<RecommendedWatcher>,
    fs_event_receiver: Receiver<notify::Result<notify::Event>>,
    fs_event_sender: Sender<notify::Result<notify::Event>>,
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
    icon_req_sender: Sender<PathBuf>,                                    // UI → Worker
    icon_res_receiver: Receiver<(PathBuf, Vec<u8>, u32, u32)>,           // Worker → UI
    loading_icons: HashSet<PathBuf>,                                     // Tracking in-progress
    
    // NOTIFICATION SYSTEM (toast messages)
    notifications: mtt_file_manager::application::NotificationManager,
}

impl ImageViewerApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let ctx = cc.egui_ctx.clone();
        
        // 1. Canais para comunicação Workers → UI
        let (file_entry_sender, file_entry_receiver) = mpsc::channel::<(usize, Vec<FileEntry>)>();
        
        // COVER WORKER: Worker Ãºnico para processar capas de pasta
        let (cover_req_tx, cover_req_rx) = mpsc::channel::<PathBuf>();  // UI â†’ Worker
        let (cover_res_tx, cover_res_rx) = mpsc::channel();             // Worker â†’ UI
        let (fs_tx, fs_rx) = mpsc::channel();
        
        // Spawna WORKER THREAD: fica em loop processando fila
        std::thread::spawn(move || {
            // Loop infinito: consome requisiÃ§Ãµes da fila
            while let Ok(folder_path) = cover_req_rx.recv() {
                // Executa busca (funÃ§Ã£o jÃ¡ existente)
                let cover = find_first_image_in_folder(&folder_path);
                
                // Devolve resultado para UI thread
                let _ = cover_res_tx.send((folder_path, cover));
            }
        });

        // --- SISTEMA DE THUMBNAILS (WORKER POOL OTIMIZADO) ---
        let (img_tx, img_rx) = mpsc::channel();
        let (req_tx, req_rx) = mpsc::channel::<(PathBuf, usize)>();
        let shared_req_rx = Arc::new(std::sync::Mutex::new(req_rx));
        let shared_gen = Arc::new(AtomicUsize::new(0));

        // 4 threads: equilíbrio ideal entre SSD e HDD USB
        use mtt_file_manager::workers::thumbnail_worker::spawn_thumbnail_workers;
        spawn_thumbnail_workers(shared_req_rx, img_tx, ctx.clone(), shared_gen.clone());
        
        // --- ASYNC ICON WORKER (single thread, evita I/O bloqueante) ---
        let (icon_req_tx, icon_req_rx) = mpsc::channel::<PathBuf>();
        let (icon_res_tx, icon_res_rx) = mpsc::channel();
        let icon_ctx = ctx.clone();
        
        std::thread::spawn(move || {
            use mtt_file_manager::infrastructure::windows::extract_file_icon_by_path;
            use mtt_file_manager::domain::file_entry::IconSize;
            
            while let Ok(path) = icon_req_rx.recv() {
                if let Ok((pixels, width, height)) = extract_file_icon_by_path(&path, IconSize::Large) {
                    let _ = icon_res_tx.send((path, pixels, width, height));
                    icon_ctx.request_repaint();
                }
            }
        });
        
        let disks = get_all_drives();
        
        let mut app = Self {
            current_path: PATH_PADRAO.to_string(),
            thumbnail_req_sender: req_tx,
            image_receiver: img_rx,
            items: Arc::new(Vec::new()),
            texture_cache: LruCache::new(NonZeroUsize::new(CACHE_SIZE).unwrap()),
            loading_set: HashSet::new(),
            // Async loading
            file_entry_receiver,
            file_entry_sender,
            is_loading_folder: false,
            // Cover Worker
            cover_worker_sender: cover_req_tx,
            cover_worker_receiver: cover_res_rx,
            scanned_folders: HashSet::new(),
            icon_cache: LruCache::new(NonZeroUsize::new(ICON_CACHE_SIZE).unwrap()),
            folder_icon_texture: None,
            computer_icon: None,
            drive_icon_cache: LruCache::new(NonZeroUsize::new(10).unwrap()),  // Poucos drives
            // Sorting - padrÃ£o: Nome, Ascendente
            sort_mode: SortMode::Name,
            sort_descending: false,
            // View mode: Grid por padrÃ£o
            view_mode: ViewMode::Grid,
            // Selection & Preview
            selected_file: None,
            show_preview_panel: true,  // Mostrar por padrÃ£o
            is_computer_view: false,
            // Navigation - comeÃ§a com path inicial no histÃ³rico
            navigation_history: vec![PATH_PADRAO.to_string()],
            history_index: 0,
            path_input: PATH_PADRAO.to_string(),
            disks,
            thumbnail_size: 128.0,  // Default zoom
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
        };
        
        // Inicia monitoramento inicial
        app.watch_current_folder();
        
        app.load_folder();
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
                let path = item.path.to_string_lossy().to_string();
                let from_vec = to_win32_path(&path);

                let mut op = SHFILEOPSTRUCTW {
                    hwnd: HWND(std::ptr::null_mut()),
                    wFunc: FO_DELETE,
                    pFrom: PCWSTR(from_vec.as_ptr()),
                    pTo: PCWSTR(std::ptr::null()),
                    fFlags: (FOF_ALLOWUNDO | FOF_WANTNUKEWARNING).0 as u16,
                    ..Default::default()
                };

                unsafe {
                    let result = SHFileOperationW(&mut op);
                    if result == 0 {
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
            self.load_folder();
        }
    }
    
    // ===== CLIPBOARD OPERATIONS (Ctrl+C, Ctrl+X, Ctrl+V) =====
    
    /// Copiar: Guarda o path do arquivo selecionado na memória
    fn command_copy(&mut self) {
        if let Some(idx) = self.selected_item {
            if let Some(item) = self.items.get(idx) {
                self.clipboard_file = Some(item.path.clone());
                self.clipboard_op = Some(ClipboardOp::Copy);
            }
        }
    }
    
    /// Recortar: Guarda o path do arquivo selecionado com flag de movimento
    fn command_cut(&mut self) {
        if let Some(idx) = self.selected_item {
            if let Some(item) = self.items.get(idx) {
                self.clipboard_file = Some(item.path.clone());
                self.clipboard_op = Some(ClipboardOp::Move);
            }
        }
    }
    
    /// Colar: Executa SHFileOperationW para copiar ou mover o arquivo
    fn command_paste(&mut self) {
        // 1. Validação: tem algo para colar?
        let src_path = match &self.clipboard_file {
            Some(p) => p.clone(),
            None => { return; }
        };
        
        // 2. Determina pasta de destino: usa target_path do menu de contexto se disponível e válido,
        // senão usa current_path (compatibilidade com atalhos de teclado)
        let dest_folder = if let Some(target) = &self.context_menu.target_path {
            // Verifica se o target ainda existe (não foi deletado)
            if target.exists() && target.is_dir() {
                target.clone()
            } else {
                // Se o target não existe mais, usa current_path e limpa o target
                self.context_menu.target_path = None;
                PathBuf::from(&self.current_path)
            }
        } else {
            PathBuf::from(&self.current_path)
        };
        
        // 3. Verifica se o arquivo de origem já existe na pasta de destino
        if let Some(file_name) = src_path.file_name() {
            let dest_file = dest_folder.join(file_name);
            if dest_file.exists() && dest_file != src_path {
                // O Windows mostrará diálogo de substituição, mas podemos prevenir operação redundante
                // Se for mover para a mesma pasta (mesmo arquivo), não faz nada
                if let Some(ClipboardOp::Move) = self.clipboard_op {
                    if src_path.parent() == Some(&dest_folder) {
                        return;
                    }
                }
            }
        }
        
        // 4. Evita mover para a mesma pasta (redundante)
        if let Some(ClipboardOp::Move) = self.clipboard_op {
            if src_path.parent() == Some(&dest_folder) {
                return;
            }
        }
        
        // 5. Prepara strings para Windows API (double-null terminated)
        let mut from_vec: Vec<u16> = src_path.to_string_lossy().encode_utf16().collect();
        from_vec.push(0);
        from_vec.push(0);
        
        let mut to_vec: Vec<u16> = dest_folder.to_string_lossy().encode_utf16().collect();
        to_vec.push(0);
        to_vec.push(0);
        
        // 6. Define operação (FO_COPY ou FO_MOVE)
        let w_func = match self.clipboard_op {
            Some(ClipboardOp::Move) => FO_MOVE,
            _ => FO_COPY,
        };
        
        let mut op = SHFILEOPSTRUCTW {
            hwnd: HWND(std::ptr::null_mut()),
            wFunc: w_func,
            pFrom: PCWSTR(from_vec.as_ptr()),
            pTo: PCWSTR(to_vec.as_ptr()),
            fFlags: (FOF_ALLOWUNDO).0 as u16,
            ..Default::default()
        };
        
        // 7. Executa operação
        unsafe {
            let result = SHFileOperationW(&mut op);
            
            if result == 0 {
                // Se foi Recortar, limpa o clipboard
                if let Some(ClipboardOp::Move) = self.clipboard_op {
                    self.clipboard_file = None;
                    self.clipboard_op = None;
                }
                // Recarrega a pasta para ver o resultado
                self.load_folder();
            }
        }
        
        // 8. Limpa o context_menu.target_path após a operação
        self.context_menu.target_path = None;
    }
    
    // Helper para botÃµes "Toggle" (que ficam acesos se selecionados)
    fn toggle_icon_button(&self, ui: &mut egui::Ui, icon: &str, active: bool, tooltip: &str) -> egui::Response {
        let color = if active { egui::Color32::from_rgb(0, 120, 215) } else { ui.visuals().text_color() };
        
        let rich_text = egui::RichText::new(icon)
            .family(egui::FontFamily::Name("icons".into()))
            .size(22.0)
            .color(color);

        // Removemos o .fill(bg) para retirar o "glow" azul
        let btn = egui::Button::new(rich_text).frame(false);
        ui.add(btn).on_hover_text(tooltip)
    }

    /// Filtra itens baseado na query de busca
    fn filter_items(&mut self) {
        if self.search_query.is_empty() {
            self.items = Arc::new(self.all_items.clone());
        } else {
            let query = self.search_query.to_lowercase();
            self.items = Arc::new(self.all_items.iter()
                .filter(|item| item.name.to_lowercase().contains(&query))
                .cloned()
                .collect());
        }
        self.total_items = self.items.len();
    }
    
    /// Ordena itens baseado no modo atual (mantém pastas sempre primeiro)
    fn sort_items(&mut self) {
        // Clone interno para mutação, depois wrap em novo Arc
        let mut items_vec = (*self.items).clone();
        items_vec.sort_by(|a, b| {
            // 1. Pastas sempre primeiro (a menos que ambos sejam pastas ou ambos arquivos)
            if a.is_dir != b.is_dir {
                return if a.is_dir {
                    Ordering::Less
                } else {
                    Ordering::Greater
                };
            }
            
            // 2. Ordena por modo selecionado (Smart Sorting com natord)
            let ordering = match self.sort_mode {
                SortMode::Name => natord::compare(&a.name.to_lowercase(), &b.name.to_lowercase()),
                SortMode::Date => a.modified.cmp(&b.modified),
                SortMode::Size => a.size.cmp(&b.size),
                SortMode::Type => {
                    // Sort by file extension, then by name
                    let ext_a = a.path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                    let ext_b = b.path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                    match ext_a.cmp(&ext_b) {
                        std::cmp::Ordering::Equal => natord::compare(&a.name.to_lowercase(), &b.name.to_lowercase()),
                        other => other,
                    }
                }
            };
            
            // 3. Inverte se descending está ativo
            if self.sort_descending {
                ordering.reverse()
            } else {
                ordering
            }
        });
        self.items = Arc::new(items_vec);
    }
    
    /// Requisita scan assÃ­ncrono de uma pasta para descobrir primeira imagem.
    /// OTIMIZADO: Envia mensagem para worker Ãºnico (zero overhead de threads)
    fn request_folder_scan(&self, folder_path: PathBuf) {
        // Apenas envia para fila - worker processa em background
        let _ = self.cover_worker_sender.send(folder_path);
    }
    
    fn load_folder(&mut self) {
        self.generation += 1; // Incrementa a geração local
        self.current_generation.store(self.generation, AtomicOrdering::Relaxed); // Sincroniza com workers
        
        // 1. Limpeza de Estado (UI Thread)
        self.items = Arc::new(Vec::new());  // Novo Arc vazio (antigo é dropped automaticamente)
        self.all_items.clear();  // Limpa backup mestre tambÃ©m
        self.texture_cache.clear();
        self.loading_set.clear();
        self.scanned_folders.clear();
        self.selected_item = None;
        self.is_loading_folder = true;
        self.total_items = 0;
        
        let my_gen = self.generation;
        let gen_clone = self.current_generation.clone();
        let current_path = self.current_path.clone();
        let file_entry_sender = self.file_entry_sender.clone();
        let ctx = self.ui_ctx.clone();
        
        // STREAMING BATCH LOADING: Envia lotes de 250 itens progressivamente
        std::thread::spawn(move || {
            // Buffer para envio em lotes
            let mut batch = Vec::with_capacity(250);
            
            // Prepara busca Win32
            let search_path = if current_path.ends_with('\\') {
                format!("{}*", current_path)
            } else {
                format!("{}\\*", current_path)
            };
            let wide_path: Vec<u16> = search_path.encode_utf16().chain(std::iter::once(0)).collect();
            let mut find_data = WIN32_FIND_DATAW::default();

            unsafe {
                if let Ok(handle) = FindFirstFileW(PCWSTR(wide_path.as_ptr()), &mut find_data) {
                    loop {
                        // Verifica se a geração mudou -> Aborta scan antigo
                        if gen_clone.load(AtomicOrdering::Relaxed) != my_gen { break; }

                        let len = find_data.cFileName.iter().position(|&c| c == 0).unwrap_or(find_data.cFileName.len());
                        let filename = std::ffi::OsString::from_wide(&find_data.cFileName[0..len])
                            .to_string_lossy()
                            .into_owned();

                        if filename != "." && filename != ".." {
                            let attrs = find_data.dwFileAttributes;
                            
                            // Filtros: hidden/system files
                            let is_hidden = (attrs & FILE_ATTRIBUTE_HIDDEN.0) != 0;
                            let is_system = (attrs & FILE_ATTRIBUTE_SYSTEM.0) != 0;
                            let is_special = matches!(filename.to_lowercase().as_str(),
                                "desktop.ini" | "thumbs.db" | "$recycle.bin" | "system volume information"
                                // Re-adicionado "System Volume Information" para garantir compatibilidade
                            );
                            
                            if !is_hidden && !is_system && !is_special && !filename.starts_with('.') {
                                let is_dir = (attrs & FILE_ATTRIBUTE_DIRECTORY.0) != 0;
                                let full_path = PathBuf::from(&current_path).join(&filename);

                                let size = if is_dir { 
                                    0 
                                } else {
                                    ((find_data.nFileSizeHigh as u64) << 32) | (find_data.nFileSizeLow as u64)
                                };

                                let ft = find_data.ftLastWriteTime;
                                let windows_ticks = ((ft.dwHighDateTime as u64) << 32) | (ft.dwLowDateTime as u64);
                                let modified = if windows_ticks > 116444736000000000 {
                                    (windows_ticks - 116444736000000000) / 10_000_000
                                } else {
                                    0
                                };

                                let entry = FileEntry {
                                    path: full_path,
                                    name: filename,
                                    is_dir,
                                    size,
                                    modified,
                                    folder_cover: None,  // Lazy load
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

            // Envia o restante (Ãºltimo lote) se sobrou algo e a geraÃ§Ã£o ainda Ã© vÃ¡lida
            if !batch.is_empty() && gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
                let _ = file_entry_sender.send((my_gen, batch));
                ctx.request_repaint();
            }
            
            // Envia vetor VAZIO para sinalizar FIM do carregamento (apenas se a geraÃ§Ã£o for a mesma)
            if gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
                let _ = file_entry_sender.send((my_gen, Vec::new()));
                ctx.request_repaint();
            }
        });
    }
    
    /// Navega para um caminho, adicionando ao histÃ³rico (corta histÃ³rico futuro)
    fn navigate_to(&mut self, path: &str) {
        // Se jÃ¡ estamos nesse caminho, nÃ£o faz nada
        if self.current_path == path {
            return;
        }
        
        // Corta histÃ³rico "futuro" (se voltamos e navegamos para outro lugar)
        if self.history_index < self.navigation_history.len().saturating_sub(1) {
            self.navigation_history.truncate(self.history_index + 1);
        }
        
        // Adiciona novo caminho ao histÃ³rico
        self.navigation_history.push(path.to_string());
        self.history_index = self.navigation_history.len() - 1;
        
        self.current_path = path.to_string();
        self.path_input = path.to_string();
        
        // Limpa o context_menu.target_path para garantir sincronia com a pasta atual
        self.context_menu.target_path = None;
        
        // ATUALIZA O VIGIA
        self.watch_current_folder();
        
        self.load_folder();
    }
    
    /// Volta no histÃ³rico (sem adicionar ao histÃ³rico)
    fn go_back(&mut self) {
        if self.can_go_back() {
            self.history_index -= 1;
            self.current_path = self.navigation_history[self.history_index].clone();
            self.path_input = self.current_path.clone();
            self.watch_current_folder();  // Atualiza o watcher
            self.load_folder();
        }
    }
    
    /// AvanÃ§a no histÃ³rico
    fn go_forward(&mut self) {
        if self.can_go_forward() {
            self.history_index += 1;
            self.current_path = self.navigation_history[self.history_index].clone();
            self.path_input = self.current_path.clone();
            self.watch_current_folder();  // Atualiza o watcher
            self.load_folder();
        }
    }
    
    /// Navega para "Este Computador" view
    fn navigate_to_computer(&mut self) {
        // Update history
        if self.history_index < self.navigation_history.len() {
            self.navigation_history.truncate(self.history_index + 1);
        }
        self.navigation_history.push(self.current_path.clone());
        self.history_index = self.navigation_history.len();
        
        // Set computer view
        self.current_path = "Este Computador".to_string();
        self.is_computer_view = true;
        self.path_input = "Este Computador".to_string();
        
        // Clear items for computer view
        self.items = Arc::new(Vec::new());
        self.all_items.clear();
        self.selected_item = None;
        self.selected_file = None;
        self.total_items = self.disks.len();
    }
    
    /// Sobe um nÃ­vel (adiciona ao histÃ³rico)
    fn go_up_one_level(&mut self) {
        if let Some(parent) = Path::new(&self.current_path).parent() {
            let parent_str = parent.to_string_lossy().to_string();
            if !parent_str.is_empty() {
                self.navigate_to(&parent_str);
            }
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

        let watcher_result = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
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
                        let result = SHFileOperationW(&mut op);
                        if result == 0 {
                            // Sucesso: Recarrega a pasta para atualizar a UI
                            self.load_folder();
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
        if let Some(texture) = self.icon_cache.get(&cache_key) {
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
                    egui::TextureOptions::LINEAR,
                );
                
                let cloned = texture.clone();
                self.icon_cache.put(cache_key, texture);
                Some(cloned)
            }
            Err(_) => None,  // Fallback: sem icone
        }
    }
    
    /// Garante que Ã­cone de pasta estÃ¡ carregado.
    fn ensure_folder_icon(&mut self, ctx: &egui::Context) {
        if self.folder_icon_texture.is_some() {
            return; // JÃ¡ carregado
        }
        
// Windows usa Ã­cone especial para pastas
        let thumbnail_size = self.thumbnail_size;
        let icon_size = if thumbnail_size < 100.0 {
            IconSize::Small
        } else {
            IconSize::Large
        };
        
        match windows_infra::extract_folder_icon(icon_size) {
            Ok((rgba_data, width, height)) => {
                let texture = ctx.load_texture(
                    "folder_icon",
                    egui::ColorImage::from_rgba_unmultiplied(
                        [width as usize, height as usize],
                        &rgba_data,
                    ),
                    egui::TextureOptions::LINEAR,
                );
                self.folder_icon_texture = Some(texture);
            }
            Err(_) => {
                // Fallback: mantÃ©m emoji
            }
        }
    }
    
    /// Garante que Ã­cone de "Este Computador" estÃ¡ carregado.
    fn ensure_computer_icon(&mut self, ctx: &egui::Context) {
        if self.computer_icon.is_some() {
            return;
        }
        
        if let Ok((data, width, height)) = windows_infra::extract_computer_icon(IconSize::Small) {
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [width as usize, height as usize],
                &data,
            );
            
            self.computer_icon = Some(ctx.load_texture(
                "computer_icon",
                image,
                egui::TextureOptions::LINEAR,
            ));
        }
    }
    
    /// Processa mensagens que chegam dos canais de workers
    fn process_incoming_messages(&mut self, ctx: &egui::Context) {
        // 1. CHECK DE REFRESH MANUAL (F5)
        if ctx.input(|i| i.key_pressed(egui::Key::F5)) {
            self.load_folder();
        }


        // 2. CHECK DE AUTO-REFRESH (WATCHER)
        while let Ok(event) = self.fs_event_receiver.try_recv() {
            match event {
                Ok(_) => self.pending_auto_reload = true,
                Err(e) => eprintln!("Erro de watch: {:?}", e),
            }
        }

        // Executa reload apenas quando debounce permitir
        if self.pending_auto_reload {
            let elapsed = self.last_auto_reload.elapsed();
            if elapsed > Duration::from_millis(500) {
                // VALIDA SE O PATH ATUAL AINDA EXISTE (pode ter sido renomeado/deletado)
                if Path::new(&self.current_path).exists() {
                    self.load_folder();
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
                
                // Reaplica filtro e ordenaÃ§Ã£o incrementalmente
                self.filter_items(); 
                self.sort_items();
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
                    folder_updates = true;
                    
                    // Requisita thumbnail se necessário
                    if !self.texture_cache.contains(&cover) && !self.loading_set.contains(&cover) {
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
                    egui::TextureOptions::LINEAR,
                );
                self.item_icon_loader.icon_cache.put(cache_key, texture);
            }
        }
        
        // 3. Individual thumbnails
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
                self.loading_set.remove(&thumbnail_data.path);
                
                let texture = ctx.load_texture(
                    thumbnail_data.path.to_string_lossy().to_string(),
                    egui::ColorImage::from_rgba_unmultiplied(
                        [thumbnail_data.width as usize, thumbnail_data.height as usize],
                        &thumbnail_data.image_data,
                    ),
                    egui::TextureOptions::LINEAR,
                );
                
                self.texture_cache.put(thumbnail_data.path, texture);
            }
        }

        
        if received_any {
            ctx.request_repaint();
        }
    }
    
    // --- DETALHES (LIST VIEW) ---
    fn render_list_view(&mut self, ui: &mut egui::Ui) {
        use mtt_file_manager::ui::views::{list_view, ListViewContext, ListViewOperations};
        
        // Extrair dados necessários para evitar múltiplos borrows
        let items = self.items.clone(); // Clone para evitar borrow
        let selected_item = self.selected_item;
        let selected_file = self.selected_file.clone();
        let sort_mode = self.sort_mode;
        let sort_descending = self.sort_descending;
        let renaming_state = self.renaming_state.clone();
        let focus_rename = self.focus_rename;
        let folder_icon_texture = self.folder_icon_texture.clone();
        let computer_icon = self.computer_icon.clone();
        
        // Criar contexto com referências mutáveis separadas
        let mut ctx = ListViewContext {
            items: &items,
            selected_item,
            selected_file: selected_file.as_ref(),
            sort_mode,
            sort_descending,
            renaming_state: renaming_state.clone(),
            focus_rename,
            texture_cache: &mut self.texture_cache,
            loading_set: &mut self.loading_set,
            scanned_folders: &mut self.scanned_folders,
            folder_icon_texture: folder_icon_texture.as_ref(),
            computer_icon: computer_icon.as_ref(),
            drive_icon_cache: &mut self.drive_icon_cache,
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
            
            fn rename_with_shell(&mut self, idx: usize) {
                self.actions.push(ListAction::RenameWithShell(idx));
            }
            
            fn get_or_load_icon(
                &mut self,
                _ctx: &egui::Context,
                _path: &std::path::Path,
            ) -> Option<egui::TextureHandle> {
                // Não podemos chamar self.app.get_or_load_icon aqui
                // Vamos retornar None e lidar com isso de outra forma
                None
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
        
        // Processar ações (bloqueadas durante renomeação)
        let is_renaming = self.renaming_state.is_some();
        match action {
            Some(list_view::ListViewAction::Click(idx)) if !is_renaming => {
                self.selected_item = Some(idx);
                if let Some(item) = self.items.get(idx) {
                    self.selected_file = Some(item.clone());
                    
                    // Trigger thumbnail load for sidebar preview
                    if !item.is_dir {
                        if !self.texture_cache.contains(&item.path) && !self.loading_set.contains(&item.path) {
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
                self.selected_item = Some(idx);
                if let Some(item) = self.items.get(idx) {
                    self.selected_file = Some(item.clone());
                    self.context_menu.open(
                        ui.ctx().pointer_latest_pos().unwrap_or(egui::Pos2::ZERO),
                        Some(idx),
                        Some(item.path.clone()),
                        false
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
        let cols = ((available_w - padding) / (item_w + padding)).floor().max(1.0) as usize;
        
        // Keyboard navigation (ONLY when not renaming)
        if self.renaming_state.is_none() {
            let current_index = self.items.iter().position(|x| 
                self.selected_file.as_ref().map_or(false, |f| f.path == x.path)
            );
            
            let mut new_index = None;
            if ui.input(|i| i.key_pressed(egui::Key::ArrowRight)) { 
                new_index = current_index.map(|idx| idx + 1).or(Some(0)); 
            }
            else if ui.input(|i| i.key_pressed(egui::Key::ArrowLeft)) { 
                new_index = current_index.map(|idx| idx.saturating_sub(1)); 
            }
            else if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) { 
                new_index = current_index.map(|idx| idx + cols).or(Some(0)); 
            }
            else if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) { 
                new_index = current_index.map(|idx| idx.saturating_sub(cols)); 
            }

            if let Some(idx) = new_index {
                let clamped = idx.min(self.items.len().saturating_sub(1));
                if let Some(item) = self.items.get(clamped) {
                    self.selected_file = Some(item.clone());
                    self.selected_item = Some(clamped);
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
        let folder_icon_texture = self.folder_icon_texture.clone();
        let computer_icon = self.computer_icon.clone();
        
        // Criar contexto com referências mutáveis separadas
        let mut ctx = GridViewContext {
            items: &items,
            selected_item,
            selected_file: selected_file.as_ref(),
            thumbnail_size,
            last_grid_cols,
            renaming_state: renaming_state.clone(),
            focus_rename,
            texture_cache: &mut self.texture_cache,
            loading_set: &mut self.loading_set,
            scanned_folders: &mut self.scanned_folders,
            folder_icon_texture: folder_icon_texture.as_ref(),
            computer_icon: computer_icon.as_ref(),
            drive_icon_cache: &mut self.drive_icon_cache,
            item_icon_loader: &mut self.item_icon_loader,
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
            
            fn rename_with_shell(&mut self, idx: usize) {
                self.actions.push(GridAction::RenameWithShell(idx));
            }
            
            fn get_or_load_icon(
                &mut self,
                _ctx: &egui::Context,
                _path: &std::path::Path,
            ) -> Option<egui::TextureHandle> {
                // Não podemos chamar self.app.get_or_load_icon aqui
                // Vamos retornar None e lidar com isso de outra forma
                None
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
                self.selected_item = Some(idx);
                if let Some(item) = self.items.get(idx) {
                    self.selected_file = Some(item.clone());
                    self.context_menu.open(
                        ui.ctx().pointer_latest_pos().unwrap_or(egui::Pos2::ZERO),
                        Some(idx),
                        Some(item.path.clone()),
                        false
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
        let is_renaming = self.renaming_state.as_ref().map_or(false, |(i, _)| *i == idx);
        
        // Para evitar conflitos de borrow, coletamos as operações pendentes
        // e executamos depois de renderizar
        let mut pending_thumbnail_loads: Vec<std::path::PathBuf> = Vec::new();
        let mut pending_folder_scans: Vec<std::path::PathBuf> = Vec::new();
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
                texture_cache: &mut self.texture_cache,
                icon_loader: &mut self.item_icon_loader,
                scanned_folders: &mut self.scanned_folders,
                loading_set: &mut self.loading_set,
            };
            
            // Create simple ops struct that collects operations
            struct SimpleOps<'a> {
                thumbnail_loads: &'a mut Vec<std::path::PathBuf>,
                folder_scans: &'a mut Vec<std::path::PathBuf>,
                pending_rename: &'a mut Option<usize>,
            }
            
            impl<'a> mtt_file_manager::ui::components::item_slot::ItemSlotOperations for SimpleOps<'a> {
                fn request_thumbnail_load(&mut self, path: std::path::PathBuf) {
                    self.thumbnail_loads.push(path);
                }
                
                fn request_folder_scan(&mut self, path: std::path::PathBuf) {
                    self.folder_scans.push(path);
                }
                
                fn rename_item(&mut self, idx: usize) {
                    *self.pending_rename = Some(idx);
                }
            }
            
            let mut ops = SimpleOps {
                thumbnail_loads: &mut pending_thumbnail_loads,
                folder_scans: &mut pending_folder_scans,
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
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        
        // --- DETECÇÃO DE COMANDOS DE SISTEMA (Clipboard) ---
        // O egui traduz Ctrl+C → Event::Copy, Ctrl+X → Event::Cut, Ctrl+V → Event::Paste
        // Isso funciona porque são eventos do SO, não teclas interceptadas.
        
        if self.renaming_state.is_none() {
            // Capturar eventos de clipboard do sistema
            let mut do_copy = false;
            let mut do_cut = false;
            let mut do_paste = false;
            
            ctx.input(|i| {
                for event in &i.events {
                    match event {
                        egui::Event::Copy => { do_copy = true; },
                        egui::Event::Cut => { do_cut = true; },
                        egui::Event::Paste(_) => { do_paste = true; },
                        _ => {}
                    }
                }
            });
            
            // Executar ações de clipboard
            if do_copy { self.command_copy(); }
            if do_cut { self.command_cut(); }
            if do_paste { self.command_paste(); }
            
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
                    &self.texture_cache,
                );
                match action {
                    StatusBarAction::SortChanged => self.sort_items(),
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
                
                if self.icon_button(ui, ICON_ARROW_UP, "Subir um nível").clicked() && !is_renaming {
                    self.go_up_one_level();
                }
                
                if self.icon_button(ui, ICON_REFRESH, "Recarregar").clicked() && !is_renaming {
                    self.load_folder();
                }

                ui.separator();

                // Botão de Nova Pasta mais visível (agora sem fundo para combinar)
                let btn_text = egui::RichText::new(format!("+ {}", ICON_FOLDER_ADD))
                    .family(egui::FontFamily::Name("icons".into()))
                    .size(22.0);
                
                let btn = egui::Button::new(btn_text).frame(false);
                if ui.add(btn).on_hover_text("Criar Nova Pasta (Ctrl+Shift+N)").clicked() && !is_renaming {
                    self.create_new_folder();
                }

                ui.separator();
                
                if self.icon_button(ui, ICON_HOME, "Ir para C:\\").clicked() && !is_renaming {
                    self.navigate_to("C:\\");
                }

                // 2. ELEMENTOS DA DIREITA (DIREITA -> ESQUERDA)
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(4.0);
                    
                    // Zoom
                    ui.add_sized(
                        egui::vec2(80.0, 20.0),
                        egui::Slider::new(&mut self.thumbnail_size, 64.0..=256.0).show_value(false)
                    );
                    ui.label("Zoom");
                    
                    ui.separator();

                    // Detalhes (Antigo Preview)
                    if self.toggle_icon_button(ui, ICON_DETAILS, self.show_preview_panel, "Detalhes").clicked() {
                        self.show_preview_panel = !self.show_preview_panel;
                    }

                    ui.separator();

                    // Modo de Visualização
                    if self.toggle_icon_button(ui, ICON_LIST, self.view_mode == ViewMode::List, "Lista").clicked() {
                        self.view_mode = ViewMode::List;
                    }
                    if self.toggle_icon_button(ui, ICON_GRID, self.view_mode == ViewMode::Grid, "Grade").clicked() {
                        self.view_mode = ViewMode::Grid;
                    }

                    ui.separator();

                    // Ordenação
                    let sort_symbol = if self.sort_descending { "▾" } else { "▴" };
                    if ui.button(sort_symbol).on_hover_text("Inverter Ordem").clicked() {
                        self.sort_descending = !self.sort_descending;
                        self.sort_items();
                    }

                    egui::ComboBox::from_id_salt("sort_mode")
                        .selected_text(match self.sort_mode {
                            SortMode::Name => "Nome",
                            SortMode::Date => "Data",
                            SortMode::Size => "Tamanho",
                            SortMode::Type => "Tipo",
                        })
                        .show_ui(ui, |ui| {
                            if ui.selectable_value(&mut self.sort_mode, SortMode::Name, "Nome").clicked() { self.sort_items(); }
                            if ui.selectable_value(&mut self.sort_mode, SortMode::Date, "Data").clicked() { self.sort_items(); }
                            if ui.selectable_value(&mut self.sort_mode, SortMode::Size, "Tamanho").clicked() { self.sort_items(); }
                            if ui.selectable_value(&mut self.sort_mode, SortMode::Type, "Tipo").clicked() { self.sort_items(); }
                        });

                    ui.separator();

                    // Busca
                    let search_width = 120.0;
                    let search_response = ui.add_sized(
                        egui::vec2(search_width, 22.0),
                        egui::TextEdit::singleline(&mut self.search_query)
                            .hint_text("Buscar...")
                    );
                    if search_response.changed() {
                        self.filter_items();
                        self.sort_items();
                    }
                    ui.label(egui::RichText::new(ICON_SEARCH).family(egui::FontFamily::Name("icons".into())).size(16.0));

                    ui.separator();

                    // 3. BARRA DE ENDEREÇO (OCUPA O MEIO)
                    let addr_width = ui.available_width().max(100.0);
                    let response = ui.add_sized(
                        egui::vec2(addr_width, 22.0),
                        egui::TextEdit::singleline(&mut self.path_input)
                            .hint_text("Caminho...")
                    );
                    
                    if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        let path = self.path_input.clone();
                        if Path::new(&path).exists() {
                            self.navigate_to(&path);
                        } else {
                            self.path_input = self.current_path.clone();
                        }
                    }
                });
            });
            ui.add_space(4.0);
        });
        
        // Windows 11 style sidebar (Restored)
        egui::SidePanel::left("sidebar")
            .min_width(200.0)
            .show(ctx, |ui| {
                use mtt_file_manager::ui::sidebar::{render_sidebar, SidebarContext, SidebarOperations};
                
                // Clonar dados necessários para evitar problemas de borrow
                let disks = self.disks.clone();
                let current_path = self.current_path.clone();
                let is_computer_view = self.is_computer_view;
                let computer_icon = self.computer_icon.clone();
                
                // Criar contexto para sidebar
                let mut ctx = SidebarContext {
                    disks: &disks,
                    current_path: &current_path,
                    is_computer_view,
                    computer_icon: computer_icon.as_ref(),
                    is_renaming: self.renaming_state.is_some(),
                };
                
                // Implementar operações da sidebar
                struct SidebarOps<'a> {
                    app: &'a mut ImageViewerApp,
                }
                
                impl<'a> SidebarOperations for SidebarOps<'a> {
                    fn navigate_to(&mut self, path: &str) {
                        self.app.navigate_to(path);
                    }
                    
                    fn navigate_to_computer(&mut self) {
                        self.app.navigate_to_computer();
                    }
                }
                
                let mut ops = SidebarOps { app: self };
                render_sidebar(ui, &mut ctx, &mut ops);
            });
        

        
        // Preview Pane (Windows Explorer style) - ANTES do CentralPanel
        if self.show_preview_panel {
            egui::SidePanel::right("preview_panel")
                .resizable(true)
                .default_width(300.0)
                .min_width(250.0)
                .max_width(500.0)
                .show(ctx, |ui| {
                    if let Some(file) = self.selected_file.clone() {
                        ui.heading("Detalhes");
                        ui.separator();
                        
                        // Preview de imagem/video (se houver thumbnail)
                        let _has_thumbnail = self.texture_cache.peek(&file.path).is_some();
                        let is_media = file.path.extension()
                            .and_then(|e| e.to_str())
                            .map(|ext| {
                                let ext_lower = ext.to_lowercase();
                                matches!(ext_lower.as_str(),
                                    "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" |
                                    "tiff" | "tif" | "ico" | "heic" | "heif" | "avif" |
                                    "mp4" | "mkv" | "avi" | "mov" | "wmv" | "flv" |
                                    "webm" | "m4v" | "mpg" | "mpeg" | "3gp" | "ts"
                                )
                            })
                            .unwrap_or(false);
                        
                        let texture = self.texture_cache.peek(&file.path).cloned();

                        if let (Some(tex), true) = (texture, is_media) {
                            // Mostra thumbnail de imagem/video
                            let max_preview_width = ui.available_width() - 20.0;
                            let max_preview_size = egui::vec2(max_preview_width, max_preview_width);
                            
                            ui.vertical_centered(|ui| {
                                ui.add(egui::Image::new(&tex)
                                    .max_size(max_preview_size)
                                    .fit_to_original_size(1.0)
                                    .shrink_to_fit());
                            });
                            ui.separator();
                        } else if !file.is_dir {
                            // Arquivo sem thumbnail -> mostra icone do Windows
                            // Aqui o self.get_or_load_icon pode ser chamado porque 'file' eh um clone
                            if let Some(icon) = self.get_or_load_icon(ui.ctx(), &file.path) {
                                let icon_display_size = 64.0;
                                ui.vertical_centered(|ui| {
                                    ui.add_space(20.0);
                                    ui.add(egui::Image::new(&icon)
                                        .max_size(egui::vec2(icon_display_size, icon_display_size))
                                        .maintain_aspect_ratio(true));
                                    ui.add_space(20.0);
                                });
                                ui.separator();
                            }
                        }
                        
                        // Tabela de detalhes
                        egui::Grid::new("details_grid")
                            .num_columns(2)
                            .spacing([10.0, 4.0])
                            .show(ui, |ui| {
                                ui.label("Nome:");
                                ui.add(egui::Label::new(&file.name)
                                    .wrap()
                                    .truncate());
                                ui.end_row();
                                
                                ui.label("Tamanho:");
                                ui.label(format_size(file.size));
                                ui.end_row();
                                
                                ui.label("Tipo:");
                                if file.is_dir {
                                    ui.label("Pasta");
                                } else {
                                    let ext = file.path.extension()
                                        .and_then(|e| e.to_str())
                                        .unwrap_or("Arquivo");
                                    ui.label(ext.to_uppercase());
                                }
                                ui.end_row();
                                
                                ui.label("Data:");
                                ui.label(format_date(file.modified));
                                ui.end_row();
                            });
                    } else {
                        ui.vertical_centered(|ui| {
                            ui.add_space(100.0);
                            ui.label("Selecione um arquivo");
                            ui.label("para ver detalhes");
                        });
                    }
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
                ui.centered_and_justified(|ui| { ui.label("Pasta vazia"); });
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
                        egui::vec2(16.0, 16.0)
                    );
                    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(spinner_rect), |ui| {
                        ui.spinner();
                    });
                }
            }
            
            // Detecção de clique direito na área vazia (fora dos itens)
            // Só abre menu de contexto se não houver item selecionado pelo clique direito
            if !self.context_menu.is_open && ui.input(|i| i.pointer.secondary_clicked()) {
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
                        let cols = ((available_w - padding) / (item_w + padding)).floor().max(1.0) as usize;
                        
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
                
                // Se não clicou em item, abre menu de contexto para área vazia
                if !clicked_on_item {
                    self.context_menu.open(
                        pointer_pos.unwrap_or(ui.ctx().pointer_latest_pos().unwrap_or(egui::Pos2::ZERO)),
                        None,
                        Some(PathBuf::from(&self.current_path)),
                        true
                    );
                }
            }
        });
        
        // Exibe o menu de contexto (se aberto)
        let mut context_menu = self.context_menu.clone();
        let clipboard_file = self.clipboard_file.clone();
        render_context_menu(ctx, &mut context_menu, &clipboard_file, self);
        self.context_menu = context_menu;
        
        // === TOAST NOTIFICATIONS ===
        self.notifications.cleanup();  // Remove expired notifications
        
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
            ctx.request_repaint();  // Keep animating
        }
    }
}
fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 720.0])
            .with_title("MTT File Manager"),
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
                    vec!["remix_icon".to_owned()]
                );
            }
            
            // Adiciona apenas fontes carregadas
            if !loaded_fonts.is_empty() {
                fonts.families.get_mut(&egui::FontFamily::Proportional)
                    .unwrap()
                    .extend(loaded_fonts.clone());
                
                fonts.families.get_mut(&egui::FontFamily::Monospace)
                    .unwrap()
                    .extend(loaded_fonts.clone());
            }
            
            cc.egui_ctx.set_fonts(fonts);
            
            Ok(Box::new(ImageViewerApp::new(cc)))
        }),
    )
}
