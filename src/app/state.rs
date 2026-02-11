//! Application state definition.
//!
//! This module defines the `ImageViewerApp` struct, which holds the entire state
//! of the application, including UI state, file lists, worker channels, and configuration.

use eframe::egui;
use lru::LruCache;
#[cfg(feature = "notify-watcher")]
use notify::RecommendedWatcher;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LastInput {
    Mouse,
    Keyboard,
}

use std::collections::VecDeque;
// use std::num::NonZeroUsize;
use std::path::PathBuf;
// PERFORMANCE: FxHashSet uses faster hashing for PathBuf keys
use crate::ui::cache::FxHashSet;
use std::sync::atomic::AtomicUsize;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::Instant;

use crate::application::navigation::NavigationHistory;
use crate::application::ClipboardManager;
use crate::domain::file_entry::{FileEntry, FoldersPosition, SortMode, ViewMode};
use crate::domain::thumbnail::ThumbnailData;
use crate::infrastructure::directory_cache::DirectoryCache;
use crate::infrastructure::directory_index::DirectoryIndex;
use crate::infrastructure::disk_cache::ThumbnailDiskCache;
use crate::infrastructure::windows as windows_infra;
// use crate::ui::cache::CacheManager;
use crate::ui::components::media_preview::MediaPreview;
use crate::ui::context_menu::ContextMenuState;
use crate::ui::icon_loader::IconLoader;
use crate::ui::svg_icons::SvgIconManager;
use crate::workers::thumbnail::PriorityThumbnailQueue;

use windows::Win32::Foundation::HWND;

pub struct ItemsRebuildResult {
    pub generation: usize,
    pub request_id: usize,
    pub items: Vec<FileEntry>,
    pub total_items: usize,
}

#[derive(Debug, Clone)]
pub enum FolderSizeMessage {
    Progress {
        folder_path: PathBuf,
        total_size: u64,
    },
    Complete {
        folder_path: PathBuf,
        total_size: u64,
    },
    Cancelled {
        folder_path: PathBuf,
    },
}

pub struct ImageViewerApp {
    pub current_path: String,
    pub loaded_path: String, // Tracks the last path we actually requested (prevents spam)

    // --- SISTEMA DE THUMBNAILS OTIMIZADO ---
    pub thumbnail_queue: Arc<PriorityThumbnailQueue>, // UI -> Worker Pool (Priority Queue)
    pub image_receiver: Receiver<ThumbnailData>,      // Worker Pool -> UI
    pub pending_thumbnails: VecDeque<ThumbnailData>,  // PERFORMANCE: Buffer for throttled uploads

    // File system
    pub items: Arc<Vec<FileEntry>>, // Arc para clone barato em render loops (60 FPS)

    // Async loading (evita freeze da UI ao ler metadata)
    pub file_entry_receiver: Receiver<(usize, Vec<FileEntry>)>,
    pub file_entry_sender: Sender<(usize, Vec<FileEntry>)>,
    pub is_loading_folder: bool,
    pub loading_started_at: Instant, // Track when loading started for timeout safety

    // Async rebuild (filter/sort) to keep UI smooth during heavy loads
    pub items_rebuild_sender: Sender<ItemsRebuildResult>,
    pub items_rebuild_receiver: Receiver<ItemsRebuildResult>,
    pub items_rebuild_request_id: usize,

    // COVER WORKER: Sistema de capas de pasta (Single Thread Worker)
    pub cover_worker_sender: Sender<PathBuf>, // UI → Worker: Envia pasta para processar
    pub cover_worker_receiver: Receiver<(PathBuf, Option<PathBuf>)>, // Worker → UI: Resultado
    pub scanned_folders: LruCache<PathBuf, ()>, // Cache: evita re-scan (LRU bounded)

    // FOLDER PREVIEW WORKER: Native Windows Shell folder previews (sandwich effect)
    pub folder_preview_sender: Sender<PathBuf>,
    pub folder_preview_receiver: Receiver<crate::workers::folder_preview_worker::FolderPreviewData>,

    // Cache Manager (unifica texture_cache, icon_cache, loading_set, etc.)
    pub cache_manager: crate::ui::cache::CacheManager,

    // Sorting state
    pub sort_mode: SortMode,
    pub sort_mode_computer: SortMode, // Sort mode for "Este Computador" view
    pub sort_mode_normal: SortMode,   // Sort mode for normal folder views
    pub sort_descending: bool,        // true = Z-A, Mais Novo, Maior
    pub folders_position: FoldersPosition, // First, Last, Mixed

    // Persistence Layer
    pub disk_cache: Arc<ThumbnailDiskCache>,
    pub directory_cache: Arc<DirectoryCache>,
    pub directory_index: Option<Arc<DirectoryIndex>>,

    // View Mode
    pub view_mode: ViewMode,

    // Navigation state (histórico linear)
    pub navigation: NavigationHistory,
    pub path_input: String, // Barra de endereço editável

    // UI state
    pub disks: Vec<(String, String)>, // (path, label)
    pub last_drive_refresh: Instant,
    pub last_drive_bitmask: u32,  // Fast bitmask from GetLogicalDrives() for quick change detection
    pub drive_scan_pending: bool, // Whether a background drive scan is in progress
    pub drive_scan_rx: Receiver<Vec<(String, String)>>, // Background drive scan results
    pub drive_scan_tx: Sender<Vec<(String, String)>>, // Sender cloned into background thread
    pub drive_info_rx: Receiver<Vec<(String, crate::domain::file_entry::DriveInfo)>>, // Background volume info
    pub drive_info_tx: Sender<Vec<(String, crate::domain::file_entry::DriveInfo)>>, // Sender for bg thread
    pub drive_info_cache: std::collections::HashMap<String, crate::domain::file_entry::DriveInfo>, // Persistent cache surviving navigation
    pub thumbnail_size: f32,                                                        // Zoom: 64-512
    pub selected_item: Option<usize>,
    pub selected_file: Option<FileEntry>,
    pub multi_selection: FxHashSet<PathBuf>,
    // Internal drag-and-drop state (Explorer-like item move/copy inside file list views)
    pub is_item_dragging: bool,
    pub drag_payload_paths: Vec<PathBuf>,
    pub drag_source_folder: Option<PathBuf>,
    pub drag_target_folder: Option<PathBuf>,
    pub drag_hovered_folder: Option<PathBuf>,
    pub selected_thumbnail: Option<egui::TextureHandle>, // Persistent thumbnail for preview panel
    pub selected_gif: Option<crate::ui::components::media_preview::GifPlayer>, // Local GIF for preview panel
    pub media_preview: Option<MediaPreview>, // Global media preview (video/image)
    pub media_preview_owner_tab_id: Option<usize>, // Tab that owns the current media preview
    pub selected_metadata: Option<(PathBuf, windows_infra::MediaMetadata)>,
    pub metadata_req_sender: Sender<(PathBuf, u64)>,
    pub metadata_res_receiver: Receiver<(PathBuf, u64, windows_infra::MediaMetadata)>,
    pub metadata_cache: LruCache<PathBuf, (u64, windows_infra::MediaMetadata)>,
    pub metadata_loading: FxHashSet<PathBuf>,
    pub last_metadata_refresh: Instant,
    pub last_metadata_path: Option<PathBuf>,
    pub show_preview_panel: bool,
    pub is_computer_view: bool, // Se estamos na view "Este Computador"
    pub computer_view_local_indices: Vec<usize>, // Pre-computed indices for local drives (virtualization)
    pub computer_view_network_indices: Vec<usize>, // Pre-computed indices for network drives (virtualization)
    pub is_recycle_bin_view: bool,                 // Se estamos na view da Lixeira
    pub show_virtual_drive_settings: bool,         // Modal de configuração de drives virtuais

    pub total_items: usize,

    // Search & Navigation (NEW)
    pub all_items: Vec<FileEntry>,            // Cache mestre para busca
    pub search_query: String,                 // Texto da busca
    pub last_grid_cols: usize,                // Memória para navegação vertical (teclado)
    pub generation: usize,                    // Contador local (Main Thread)
    pub current_generation: Arc<AtomicUsize>, // Contador compartilhado (Workers)
    pub ui_ctx: egui::Context, // Referência ao contexto da UI para repaints assíncronos
    // PERFORMANCE: Throttle rebuild de lista durante streaming
    pub last_items_rebuild: Instant,
    pub pending_items_rebuild: bool,
    pub pending_items_count: usize,

    // ESTADO DE RENOMEAÇÃO
    pub renaming_state: Option<(usize, String)>, // (Index, Texto Editável)
    pub focus_rename: bool,                      // Trigger para focar no input

    // SISTEMA DE WATCHER (AUTO-REFRESH)
    // Drive-wide watcher (novo - monitora drive inteiro)
    pub drive_watcher: crate::infrastructure::drive_watcher_integration::DriveWatcherManager,

    // Legacy notify-based watcher (fallback)
    #[cfg(feature = "notify-watcher")]
    pub watcher: Option<RecommendedWatcher>,
    #[cfg(feature = "notify-watcher")]
    pub fs_event_receiver: Receiver<notify::Result<notify::Event>>,
    #[cfg(feature = "notify-watcher")]
    pub fs_event_sender: Sender<notify::Result<notify::Event>>,
    pub device_event_receiver: Receiver<()>,
    pub last_auto_reload: Instant,
    pub pending_auto_reload: bool,
    pub skip_next_auto_reload: bool, // SMART DELETE: Prevent reload after direct UI update

    // CLIPBOARD (Copiar/Recortar/Colar)
    // CLIPBOARD (Copiar/Recortar/Colar)
    pub clipboard: ClipboardManager,

    // CONTEXT MENU STATE
    pub context_menu: ContextMenuState,

    // ICON LOADER PERSISTENTE (evita criar novo a cada frame)
    pub item_icon_loader: IconLoader,

    // GIF MANAGER OTIMIZADO
    pub gif_manager: crate::ui::components::gif_manager::GifManager,

    // ASYNC ICON WORKER (evita I/O bloqueante no render loop)
    pub icon_req_sender: Sender<PathBuf>, // UI → Worker
    pub icon_res_receiver: Receiver<(PathBuf, Vec<u8>, u32, u32)>, // Worker → UI
    pub loading_icons: FxHashSet<PathBuf>, // Tracking in-progress
    pub failed_icons: LruCache<PathBuf, ()>, // Icons that failed extraction (LRU bounded)

    // NOTIFICATION SYSTEM (toast messages)
    pub notifications: crate::application::NotificationManager,

    // ONEDRIVE SIDEBAR SHORTCUT
    pub onedrive_path: Option<String>, // Caminho do OneDrive (se instalado)
    pub onedrive_icon: Option<egui::TextureHandle>, // Ícone nativo do OneDrive

    // STARTUP OPTIMIZATION: Async Font Loading
    pub font_loader_rx: Option<Receiver<egui::FontDefinitions>>,

    // NAVEGAÇÃO / ADDRESS BAR (Breadcrumbs vs Edit)
    pub is_address_editing: bool,

    // SCROLL TO SELECTED (para navegação por teclado)
    pub scroll_to_selected: bool,
    pub selection_anchor: Option<usize>,

    /// Path to select and scroll to after folder reload completes (e.g., after rename)
    pub pending_select_path: Option<PathBuf>,

    // Throttle for keyboard navigation (prevents scroll desync when holding arrow keys)
    pub last_keyboard_nav: Instant,

    // SVG ICON MANAGER
    pub svg_icon_manager: SvgIconManager,

    // LAST INPUT STATE (Strict Hover Control)
    pub last_input: LastInput,

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
    pub saved_is_minimized: bool,

    // Sidebar widths persistence
    pub sidebar_left_width: f32,
    pub sidebar_right_width: f32,

    // TAB SYSTEM
    pub tab_manager: crate::tabs::TabManager,

    // FOLDER SIZE CALCULATOR (async for details panel)
    pub folder_size_req_sender: Sender<PathBuf>, // UI → Worker
    pub folder_size_res_receiver: Receiver<FolderSizeMessage>, // Worker → UI (progress + complete)
    pub folder_size_cancel: std::sync::Arc<std::sync::atomic::AtomicBool>, // Cancel current calculation
    pub folder_size_cache: LruCache<PathBuf, u64>, // Calculated sizes (LRU bounded)
    pub folder_size_loading: FxHashSet<PathBuf>,   // Currently calculating

    // RECYCLE BIN CACHE
    pub deletion_date_cache: LruCache<String, String>,

    // PERFORMANCE: Reusable buffers for grid view rendering (avoid per-item allocations)
    pub pending_ops: crate::ui::views::grid_view::PendingOperations,
    pub scroll_predictor: crate::ui::views::grid_view::ScrollPredictor,

    // Scroll offset for manual grid virtualization
    pub scroll_offset_y: f32,

    // PERFORMANCE: Visible item range for GPU upload prioritization
    // Set by grid/list view each frame; used by upload loop to prioritize on-screen items
    pub visible_index_range: Option<(usize, usize)>,

    // PERFORMANCE: Cached visible paths set to avoid per-frame allocation during scroll
    // Stores the last computed visible paths and the range that generated them
    pub visible_paths_cache: FxHashSet<PathBuf>,
    pub visible_range_cached: Option<(usize, usize)>,

    // PERFORMANCE: Scroll state tracking for adaptive GPU upload throttling
    pub last_scroll_time: Instant,
    pub last_scroll_offset: f32,
    pub frame_time_avg_ms: f32,
    pub frame_time_peak_ms: f32,
    pub fps_avg: f32,
    pub upload_budget_ms: f32,
    pub last_upload_budget_update: Instant,
    pub last_memory_maintenance: Instant,

    // INACTIVITY RECOVERY: Track when app was restored from minimized state
    // Used to throttle heavy operations (watcher events, thumbnail loads) for a few frames
    // after returning from long inactivity, preventing OneDrive-related freezes
    pub last_restore_time: Instant,
    pub minimized_duration_secs: f64,

    // PREFERENCES DEBOUNCE: Instead of writing 20+ SQLite rows immediately on every
    // state change (which blocks the UI thread with disk I/O), we set a dirty flag
    // and flush no more than once per second.
    pub preferences_dirty: bool,
    pub preferences_last_save: Instant,

    // Media player volume persistence
    pub saved_media_volume: f32,

    // Explicit scroll request for keyboard navigation
    pub scroll_request: ScrollRequest,

    // GLOBAL SEARCH (via MTT Search Service)
    pub global_search_sender: Sender<crate::workers::global_search_worker::GlobalSearchRequest>,
    pub global_search_receiver: Receiver<crate::workers::global_search_worker::GlobalSearchResponse>,
    pub global_search_query: String,
    pub global_search_results: Vec<mtt_search_protocol::SearchResultItem>,
    pub global_search_active: bool,
    pub global_search_loading: bool,
    pub global_search_available: bool,
    pub global_search_last_check: Instant,
    pub global_search_total_indexed: u64,

    // FILE OPERATION WORKER
    pub file_op_sender: Sender<crate::workers::file_operation_worker::FileOperationRequest>,
    pub file_op_res_receiver: Receiver<crate::workers::file_operation_worker::FileOperationResult>,
    pub disk_cache_invalidation_sender: Sender<Vec<PathBuf>>,
    pub prefetch_sender: Sender<crate::workers::prefetch_worker::PrefetchMessage>,
    pub predictive_sender: Sender<crate::workers::predictive_prefetch::PredictiveMessage>,
    pub idle_warmup_sender: Sender<crate::workers::idle_warmup::IdleWarmupMessage>,

    // FILE OPERATION TRACKING (suppresses watcher auto-reload during copy/move/delete)
    pub file_ops_in_progress: usize,
    /// Paths currently being deleted — shared with worker threads to cancel in-flight extractions
    pub pending_deletions: Arc<dashmap::DashMap<PathBuf, ()>>,

    // ISO MOUNTING
    pub pending_iso_mount: Option<PathBuf>,

    // Media keyboard debounce
    pub last_media_key_press: Instant,

    // List view column widths (resizable) - Regular view
    pub list_col_name_width: f32,
    pub list_col_date_width: f32,
    pub list_col_type_width: f32,
    pub list_col_size_width: f32,
    // List view column widths - OneDrive view
    pub list_col_onedrive_name_width: f32,
    pub list_col_onedrive_date_width: f32,
    pub list_col_onedrive_type_width: f32,
    pub list_col_onedrive_size_width: f32,
    pub list_col_onedrive_status_width: f32,
    // List view column widths - Computer view
    pub list_col_computer_name_width: f32,
    pub list_col_computer_total_width: f32,
    pub list_col_computer_free_width: f32,
}

impl ImageViewerApp {
    /// Check if a video is actively playing in docked mode (preview panel)
    /// Used to throttle disk I/O from thumbnails to prevent stutter during video playback
    pub fn is_video_playing_docked(&self) -> bool {
        if let Some(preview) = &self.media_preview {
            // Must be: (1) docked (not detached), (2) visible/initialized, and (3) playing
            if !preview.is_detached() && preview.is_player_visible() {
                if let Some(state) = preview.get_video_state() {
                    return state.is_playing;
                }
            }
        }
        false
    }

    pub fn is_video_docked_visible(&self) -> bool {
        if let Some(preview) = &self.media_preview {
            !preview.is_detached() && preview.is_visible()
        } else {
            false
        }
    }

    /// Check if the media player should currently capture all keyboard arrow/space input.
    /// Returns true if player is detached/fullscreen AND has focus.
    pub fn is_media_keyboard_focused(&self) -> bool {
        let preview = if let Some(p) = &self.media_preview {
            p
        } else {
            return false;
        };

        // Condition 1: Must be detached or fullscreen
        if !preview.is_detached() && !preview.is_maximized() {
            return false;
        }

        // Condition 2: Current tab must be the owner
        let active_tab_id = self.tab_manager.active().id;
        if self.media_preview_owner_tab_id != Some(active_tab_id) {
            return false;
        }

        #[cfg(target_os = "windows")]
        {
            use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
            let foreground = unsafe { GetForegroundWindow() };
            if foreground.is_invalid() {
                return false;
            }

            // Focused if either the main app or the MPV child window is in foreground
            self.native_hwnd == Some(foreground) || preview.get_hwnd() == Some(foreground)
        }
        #[cfg(not(target_os = "windows"))]
        {
            false
        }
    }

    /// Applies bounded cache cleanup when process memory is above thresholds.
    /// Keeps hot assets while avoiding long-session RAM growth.
    pub fn run_memory_maintenance(&mut self) {
        self.run_memory_maintenance_impl(false);
    }

    /// Runs memory maintenance immediately, bypassing normal periodic throttle.
    pub fn run_memory_maintenance_now(&mut self) {
        self.run_memory_maintenance_impl(true);
    }

    fn run_memory_maintenance_impl(&mut self, force: bool) {
        use std::time::Duration;

        if !force && self.last_memory_maintenance.elapsed() < Duration::from_secs(2) {
            return;
        }
        self.last_memory_maintenance = Instant::now();

        let Some(working_set_bytes) = current_working_set_bytes() else {
            return;
        };

        const SOFT_LIMIT_BYTES: u64 = 550 * 1024 * 1024;
        const HARD_LIMIT_BYTES: u64 = 700 * 1024 * 1024;

        if working_set_bytes < SOFT_LIMIT_BYTES {
            return;
        }

        let aggressive = working_set_bytes >= HARD_LIMIT_BYTES;
        let max_pending = if aggressive { 24 } else { 48 };
        let min_folder_previews_keep = self.estimated_visible_folder_previews();

        while self.pending_thumbnails.len() > max_pending {
            if let Some(old) = self.pending_thumbnails.pop_front() {
                self.cache_manager.finish_pending_upload(&old.path);
            } else {
                break;
            }
        }

        let (textures_removed, rgba_removed, folder_previews_removed) = if aggressive {
            self.cache_manager.trim_thumbnail_caches(
                96,
                64 * 1024 * 1024,
                min_folder_previews_keep.max(72),
            )
        } else {
            self.cache_manager.trim_thumbnail_caches(
                140,
                96 * 1024 * 1024,
                min_folder_previews_keep.max(120),
            )
        };

        if aggressive {
            self.directory_cache.clear();
            self.visible_paths_cache.clear();
            self.visible_range_cached = None;
        }

        // Reuse existing GIF cleanup policy (TTL + bounded memory) without forcing visible preview drop.
        self.gif_manager.cleanup(false);

        if textures_removed > 0 || rgba_removed > 0 || folder_previews_removed > 0 {
            eprintln!(
                "[MEMORY] RAM {:.1}MB -> trimmed textures={} rgba={} folder_previews={} pending={} mode={}",
                working_set_bytes as f64 / 1024.0 / 1024.0,
                textures_removed,
                rgba_removed,
                folder_previews_removed,
                max_pending,
                if aggressive { "hard" } else { "soft" }
            );
        }
    }

    fn estimated_visible_folder_previews(&self) -> usize {
        if !matches!(self.view_mode, ViewMode::Grid)
            || self.is_computer_view
            || self.is_recycle_bin_view
        {
            return 0;
        }

        let screen = self.ui_ctx.screen_rect();
        let mut central_width = screen.width()
            - self.sidebar_left_width.clamp(150.0, 500.0)
            - if self.show_preview_panel {
                self.sidebar_right_width.clamp(250.0, 500.0)
            } else {
                0.0
            };
        central_width = (central_width - 24.0).max(0.0);

        let thumbnail_size = self.thumbnail_size.max(96.0);
        let padding = 8.0;
        let cols = ((central_width - padding) / (thumbnail_size + padding))
            .floor()
            .max(1.0) as usize;

        let central_height = (screen.height() - 72.0).max(0.0);
        let row_height = thumbnail_size + 20.0 + padding;
        let rows = (central_height / row_height).ceil().max(1.0) as usize;

        cols.saturating_mul(rows.saturating_add(2)).clamp(48, 320)
    }
}

#[cfg(target_os = "windows")]
fn current_working_set_bytes() -> Option<u64> {
    use windows::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::GetCurrentProcess;

    unsafe {
        let mut counters = PROCESS_MEMORY_COUNTERS::default();
        if K32GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut counters,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        )
        .as_bool()
        {
            Some(counters.WorkingSetSize as u64)
        } else {
            None
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn current_working_set_bytes() -> Option<u64> {
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollRequest {
    None,
    EnsureVisible(usize),
}
