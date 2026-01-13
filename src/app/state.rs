//! Application state definition.
//!
//! This module defines the `ImageViewerApp` struct, which holds the entire state
//! of the application, including UI state, file lists, worker channels, and configuration.

use eframe::egui;
use lru::LruCache;
use notify::RecommendedWatcher;

use std::collections::HashSet;
// use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::Instant;

use crate::application::navigation::NavigationHistory;
use crate::application::ClipboardManager;
use crate::domain::file_entry::{FileEntry, FoldersPosition, SortMode, ViewMode};
use crate::domain::thumbnail::ThumbnailData;
use crate::infrastructure::disk_cache::ThumbnailDiskCache;
use crate::infrastructure::windows as windows_infra;
// use crate::ui::cache::CacheManager;
use crate::ui::context_menu::ContextMenuState;
use crate::ui::icon_loader::IconLoader;
use crate::ui::svg_icons::SvgIconManager;

use windows::Win32::Foundation::HWND;

pub struct ImageViewerApp {
    pub current_path: String,

    // --- SISTEMA DE THUMBNAILS OTIMIZADO ---
    pub thumbnail_req_sender: Sender<(PathBuf, usize)>, // UI -> Worker Pool
    pub image_receiver: Receiver<ThumbnailData>,        // Worker Pool -> UI

    // File system
    pub items: Arc<Vec<FileEntry>>, // Arc para clone barato em render loops (60 FPS)

    // Async loading (evita freeze da UI ao ler metadata)
    pub file_entry_receiver: Receiver<(usize, Vec<FileEntry>)>,
    pub file_entry_sender: Sender<(usize, Vec<FileEntry>)>,
    pub is_loading_folder: bool,

    // COVER WORKER: Sistema de capas de pasta (Single Thread Worker)
    pub cover_worker_sender: Sender<PathBuf>, // UI → Worker: Envia pasta para processar
    pub cover_worker_receiver: Receiver<(PathBuf, Option<PathBuf>)>, // Worker → UI: Resultado
    pub scanned_folders: HashSet<PathBuf>,    // Cache: evita re-scan

    // FOLDER PREVIEW WORKER: Native Windows Shell folder previews (sandwich effect)
    pub folder_preview_sender: Sender<PathBuf>,
    pub folder_preview_receiver: Receiver<crate::workers::folder_preview_worker::FolderPreviewData>,

    // Cache Manager (unifica texture_cache, icon_cache, loading_set, etc.)
    pub cache_manager: crate::ui::cache::CacheManager,

    // Sorting state
    pub sort_mode: SortMode,
    pub sort_descending: bool,             // true = Z-A, Mais Novo, Maior
    pub folders_position: FoldersPosition, // First, Last, Mixed

    // Persistence Layer
    pub disk_cache: Arc<ThumbnailDiskCache>,

    // View Mode
    pub view_mode: ViewMode,

    // Navigation state (histórico linear)
    pub navigation: NavigationHistory,
    pub path_input: String, // Barra de endereço editável

    // UI state
    pub disks: Vec<(String, String)>, // (path, label)
    pub last_drive_refresh: Instant,
    pub thumbnail_size: f32, // Zoom: 64-512
    pub selected_item: Option<usize>,
    pub selected_file: Option<FileEntry>,
    pub selected_thumbnail: Option<egui::TextureHandle>, // Persistent thumbnail for preview panel
    pub selected_metadata: Option<(PathBuf, windows_infra::MediaMetadata)>,
    pub metadata_req_sender: Sender<(PathBuf, u64)>,
    pub metadata_res_receiver: Receiver<(PathBuf, u64, windows_infra::MediaMetadata)>,
    pub metadata_cache: LruCache<PathBuf, (u64, windows_infra::MediaMetadata)>,
    pub metadata_loading: HashSet<PathBuf>,
    pub show_preview_panel: bool,
    pub is_computer_view: bool,    // Se estamos na view "Este Computador"
    pub is_recycle_bin_view: bool, // Se estamos na view da Lixeira

    pub total_items: usize,

    // Search & Navigation (NEW)
    pub all_items: Vec<FileEntry>,            // Cache mestre para busca
    pub search_query: String,                 // Texto da busca
    pub last_grid_cols: usize,                // Memória para navegação vertical (teclado)
    pub generation: usize,                    // Contador local (Main Thread)
    pub current_generation: Arc<AtomicUsize>, // Contador compartilhado (Workers)
    pub ui_ctx: egui::Context, // Referência ao contexto da UI para repaints assíncronos

    // ESTADO DE RENOMEAÇÃO
    pub renaming_state: Option<(usize, String)>, // (Index, Texto Editável)
    pub focus_rename: bool,                      // Trigger para focar no input

    // SISTEMA DE WATCHER (AUTO-REFRESH)
    pub watcher: Option<RecommendedWatcher>,
    pub fs_event_receiver: Receiver<notify::Result<notify::Event>>,
    pub fs_event_sender: Sender<notify::Result<notify::Event>>,
    pub device_event_receiver: Receiver<()>,
    pub last_auto_reload: Instant,
    pub pending_auto_reload: bool,

    // CLIPBOARD (Copiar/Recortar/Colar)
    // CLIPBOARD (Copiar/Recortar/Colar)
    pub clipboard: ClipboardManager,

    // CONTEXT MENU STATE
    pub context_menu: ContextMenuState,

    // ICON LOADER PERSISTENTE (evita criar novo a cada frame)
    pub item_icon_loader: IconLoader,

    // ASYNC ICON WORKER (evita I/O bloqueante no render loop)
    pub icon_req_sender: Sender<PathBuf>, // UI → Worker
    pub icon_res_receiver: Receiver<(PathBuf, Vec<u8>, u32, u32)>, // Worker → UI
    pub loading_icons: HashSet<PathBuf>,  // Tracking in-progress

    // NOTIFICATION SYSTEM (toast messages)
    pub notifications: crate::application::NotificationManager,

    // ONEDRIVE SIDEBAR SHORTCUT
    pub onedrive_path: Option<String>, // Caminho do OneDrive (se instalado)
    pub onedrive_icon: Option<egui::TextureHandle>, // Ícone nativo do OneDrive

    // NAVEGAÇÃO / ADDRESS BAR (Breadcrumbs vs Edit)
    pub is_address_editing: bool,

    // SCROLL TO SELECTED (para navegação por teclado)
    pub scroll_to_selected: bool,

    // Throttle for keyboard navigation (prevents scroll desync when holding arrow keys)
    pub last_keyboard_nav: Instant,

    // SVG ICON MANAGER
    pub svg_icon_manager: SvgIconManager,

    // Debounce for paste key (keys_down can fire multiple times)
    pub paste_key_debounce: bool,

    // Window handle for native shell interactions
    pub native_hwnd: Option<HWND>,

    // 3-stage startup: hidden -> maximize/resize -> reveal
    pub startup_tick: usize,

    // Window state persistence
    pub saved_window_width: f32,
    pub saved_window_height: f32,
    pub saved_is_maximized: bool,

    // Sidebar widths persistence
    pub sidebar_left_width: f32,
    pub sidebar_right_width: f32,

    // TAB SYSTEM
    pub tab_manager: crate::tabs::TabManager,

    // FOLDER SIZE CALCULATOR (async for details panel)
    pub folder_size_req_sender: Sender<PathBuf>, // UI → Worker
    pub folder_size_res_receiver: Receiver<(PathBuf, u64)>, // Worker → UI
    pub folder_size_cache: std::collections::HashMap<PathBuf, u64>, // Calculated sizes
    pub folder_size_loading: HashSet<PathBuf>,   // Currently calculating

    // RECYCLE BIN CACHE
    pub deletion_date_cache: LruCache<String, String>,
}
