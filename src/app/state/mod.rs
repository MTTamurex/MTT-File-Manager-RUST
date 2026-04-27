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
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::Instant;

use crate::app::drive_state::DriveState;
use crate::app::dual_panel::{ActivePanel, PanelSnapshot};
use crate::app::file_operation_state::FileOperationState;
use crate::app::folder_size_state::FolderSizeState;
use crate::app::global_search_state::GlobalSearchState;
use crate::app::layout_state::LayoutState;
use crate::app::navigation_state::{NavigationState, ThemeMode};
use crate::app::shortcuts::{ShortcutBindings, ShortcutEditorState};
use crate::application::ClipboardManager;
use crate::domain::file_entry::{FileEntry, FoldersPosition, SortMode, ViewMode};
use crate::domain::thumbnail::ThumbnailData;
use crate::infrastructure::app_state_db::AppStateDb;
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
pub struct WatcherFsProbeCacheEntry {
    pub file_system: Option<String>,
    pub is_usn: bool,
    pub probed_at: Instant,
}

pub struct ImageViewerApp {
    pub navigation_state: NavigationState,
    /// Last known modified timestamp for the currently browsed folder.
    /// Filled at navigation time from the already selected/listed folder entry
    /// to avoid blocking filesystem calls in the render loop.
    pub current_folder_modified_hint: Option<(PathBuf, u64)>,
    /// Cache of known folder modified timestamps by folder path.
    /// Used to preserve "Data modificada" in preview panel across back/forward navigation.
    /// Bounded to 500 entries via LRU eviction to prevent unbounded growth.
    pub folder_modified_hints: lru::LruCache<PathBuf, u64>,
    pub loaded_path: String, // Tracks the last path we actually requested (prevents spam)

    // --- OPTIMIZED THUMBNAIL SYSTEM ---
    pub thumbnail_queue: Arc<PriorityThumbnailQueue>, // UI -> Worker Pool (Priority Queue)
    pub image_receiver: crossbeam_channel::Receiver<ThumbnailData>, // Worker Pool -> UI
    pub pending_thumbnails: VecDeque<ThumbnailData>,  // PERFORMANCE: Buffer for throttled uploads
    /// Per-path counter of stale in-flight thumbnail results to skip.
    /// Incremented on cache eviction, decremented when a stale result is drained.
    pub thumbnail_eviction_skips: std::collections::HashMap<PathBuf, u32>,
    /// Snapshot of old items' metadata (path â†’ (modified, size)) taken before
    /// a watcher-triggered reload clears `all_items`. Used after end-of-load to
    /// detect and evict stale `texture_cache` entries for items whose content
    /// changed on disk.
    pub stale_items_snapshot: Option<std::collections::HashMap<PathBuf, (u64, u64)>>,

    // File system
    pub items: Arc<Vec<FileEntry>>, // Arc for cheap clone in render loops (60 FPS)

    // Async loading (prevents UI freeze when reading metadata)
    pub file_entry_receiver: Receiver<(usize, Vec<FileEntry>)>,
    pub file_entry_sender: Sender<(usize, Vec<FileEntry>)>,
    pub is_loading_folder: bool,
    pub loading_started_at: Instant, // Track when loading started for timeout safety

    // Async rebuild (filter/sort) to keep UI smooth during heavy loads
    pub items_rebuild_sender: Sender<ItemsRebuildResult>,
    pub items_rebuild_receiver: Receiver<ItemsRebuildResult>,
    pub items_rebuild_request_id: usize,
    pub items_rebuild_in_flight: bool,

    // COVER WORKER: Folder cover system (Single Thread Worker)
    pub cover_worker_sender: Sender<PathBuf>, // UI â†’ Worker: Sends folder to process
    pub cover_worker_receiver: Receiver<(PathBuf, Option<PathBuf>)>, // Worker â†’ UI: Result
    pub scanned_folders: LruCache<PathBuf, ()>, // Cache: avoids re-scan (LRU bounded)

    // FOLDER PREVIEW WORKER: Native Windows Shell folder previews (sandwich effect)
    pub folder_preview_sender: crossbeam_channel::Sender<PathBuf>,
    pub folder_preview_receiver: Receiver<crate::workers::folder_preview_worker::FolderPreviewData>,

    // Cache Manager (unifica texture_cache, icon_cache, loading_set, etc.)
    pub cache_manager: crate::ui::cache::CacheManager,

    // Sorting state
    pub sort_mode: SortMode,
    pub sort_mode_computer: SortMode, // Sort mode for "This PC" view
    pub sort_mode_normal: SortMode,   // Sort mode for normal folder views
    pub sort_descending: bool,        // true = Z-A, Newest, Largest
    pub folders_position: FoldersPosition, // First, Last, Mixed
    pub show_hidden_files: bool,      // Show files with FILE_ATTRIBUTE_HIDDEN
    pub show_recycle_bin: bool,       // Show Recycle Bin in Quick Access
    pub collapse_quick_access: bool,  // Collapse Quick Access section in sidebar
    pub collapse_local_disks: bool,   // Collapse Local Disks section in sidebar
    pub collapse_network_drives: bool, // Collapse Network Drives section in sidebar

    // "Normal" (unlocked) state â€” these track what unlocked folders should use.
    // When a locked folder overrides active settings, these remain unchanged.
    pub sort_descending_normal: bool,
    pub folders_position_normal: FoldersPosition,
    pub view_mode_normal: ViewMode,

    // Persistence Layer
    pub disk_cache: Arc<ThumbnailDiskCache>,
    pub app_state_db: Arc<AppStateDb>,
    pub directory_cache: Arc<DirectoryCache>,
    pub directory_dirty_registry:
        Arc<crate::infrastructure::directory_dirty_registry::DirectoryDirtyRegistry>,
    pub directory_index: Option<Arc<DirectoryIndex>>,

    // View Mode
    pub view_mode: ViewMode,

    // UI state
    pub drive_state: DriveState,
    pub thumbnail_size: f32, // Zoom: 96-512
    pub selected_item: Option<usize>,
    pub selected_file: Option<FileEntry>,
    pub multi_selection: FxHashSet<PathBuf>,
    // Internal drag-and-drop state (Explorer-like item move/copy inside file list views)
    pub is_item_dragging: bool,
    pub drag_payload_paths: Vec<PathBuf>,
    pub drag_payload_is_single_directory: bool,
    pub drag_source_folder: Option<PathBuf>,
    pub drag_target_folder: Option<PathBuf>,
    pub drag_hovered_folder: Option<PathBuf>,
    /// Cross-panel drop target: set by render_dual_panel when dragging over the inactive panel.
    /// Used as fallback in complete_item_drag when drag_target_folder is None.
    pub drag_cross_panel_target: Option<PathBuf>,
    /// Icon pre-loaded when drag starts â€” avoids blocking Shell calls in the render loop.
    pub drag_icon_cache: Option<egui::TextureHandle>,
    pub selected_thumbnail: Option<egui::TextureHandle>, // Persistent thumbnail for preview panel
    pub selected_gif: Option<crate::ui::components::media_preview::GifPlayer>, // Local GIF for preview panel
    pub media_preview: Option<MediaPreview>, // Global media preview (video/image)
    pub media_preview_owner_tab_id: Option<usize>, // Tab that owns the current media preview
    pub video_player_process: Option<std::process::Child>, // Standalone video player process handle
    pub selected_metadata: Option<(PathBuf, windows_infra::MediaMetadata)>,
    pub metadata_req_sender: Sender<(PathBuf, u64)>,
    pub metadata_res_receiver: Receiver<(PathBuf, u64, windows_infra::MediaMetadata)>,
    pub metadata_cache: LruCache<PathBuf, (u64, windows_infra::MediaMetadata)>,
    pub metadata_loading: FxHashSet<PathBuf>,
    pub live_file_size_req_sender: Sender<crate::app::live_file_size::LiveFileSizeRequest>,
    pub live_file_size_res_receiver: Receiver<crate::app::live_file_size::LiveFileSizeResponse>,
    pub live_file_size_cache: LruCache<PathBuf, (u64, u64)>,
    pub live_file_size_loading: FxHashSet<PathBuf>,
    pub last_metadata_refresh: Instant,
    pub last_metadata_path: Option<PathBuf>,
    pub show_left_sidebar: bool,
    pub show_preview_panel: bool,

    pub total_items: usize,

    // Search & Navigation (NEW)
    pub all_items: Arc<Vec<FileEntry>>, // Master cache for search
    pub search_query: String,           // Search text
    pub last_grid_cols: usize,          // Memory for vertical navigation (keyboard)
    pub generation: usize,              // Local counter (Main Thread)
    pub current_generation: Arc<AtomicUsize>, // Shared counter (Workers)
    pub ui_ctx: egui::Context,          // Reference to UI context for async repaints
    // PERFORMANCE: Throttle list rebuild during streaming
    pub last_items_rebuild: Instant,
    pub pending_items_rebuild: bool,
    pub pending_items_count: usize,
    /// When true, `all_items` will be cleared on the first incoming batch
    /// of the current generation. This allows watcher-triggered reloads to
    /// keep the old items visible until new data starts arriving.
    pub pending_all_items_clear: bool,

    // RENAME STATE
    pub renaming_state: Option<(usize, String)>, // (Index, Editable Text)
    pub focus_rename: bool,                      // Trigger to focus the input

    // SIDEBAR DRIVE RENAME (inline in sidebar, not in main view)
    /// (drive_path, editable_label) â€” active inline rename in the sidebar
    pub sidebar_renaming: Option<(String, String)>,
    pub sidebar_rename_focus: bool,

    // WATCHER SYSTEM (AUTO-REFRESH)
    // Per-folder notify-based watcher
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
    pub watcher_cooldown_until: Option<Instant>,
    /// Background OneDrive pin operations set this flag when attrib finishes,
    /// so the update loop can trigger a folder reload with fresh sync status.
    pub onedrive_pin_reload_pending: Arc<AtomicBool>,
    /// Adaptive RDCW verification for non-USN filesystems.
    /// Starts in verification mode (slow probing) and escalates to active
    /// polling only when the probe detects drift (RDCW missed events).
    pub watcher_fallback_polling: bool,
    pub watcher_fallback_fs: Option<String>,
    pub watcher_fallback_last_probe: Instant,
    pub watcher_fallback_signature: Option<u64>,
    /// Independent drift probe cadence for the visible inactive dual-panel.
    /// The OS watcher is configured for both panels, but this catches missed
    /// cross-process events without making the inactive panel focused.
    pub dual_panel_inactive_last_probe: Instant,
    /// Per-drive RDCW reliability verdict, learned during the session.
    /// `true` = RDCW confirmed unreliable (drift was detected at least once).
    /// Drives not in this map are still being verified.
    pub rdcw_unreliable_drives: std::collections::HashMap<char, bool>,
    /// Debounced folder mtime recheck: folders whose `modified` timestamp should
    /// be re-read from the filesystem after a short delay.  Windows may not
    /// update a directory's `LastWriteTime` until all file handles inside it
    /// are closed, so the first read right after a CREATE event often returns
    /// the old value.
    ///
    /// Uses a sliding-window debounce: each new event for the same folder
    /// pushes the recheck deadline forward, so rapid-fire events (downloads,
    /// torrent writes) coalesce into a single metadata read + re-sort.
    /// Each entry stores `(path, scheduled_recheck_time)`.
    pub pending_folder_mtime_recheck: Vec<(std::path::PathBuf, Instant)>,
    /// Timestamp of the last folder-mtime re-sort to enforce a cooldown and
    /// prevent excessive re-sorts during sustained write bursts.
    pub last_folder_mtime_sort: Instant,
    /// Cached filesystem probe metadata per local drive letter.
    /// Avoids repeated `GetVolumeInformationW` cost during frequent watcher reconfiguration.
    pub watcher_fs_probe_cache: std::collections::HashMap<char, WatcherFsProbeCacheEntry>,
    /// Async consistency probe for non-USN drives (avoids blocking UI thread)
    pub consistency_probe_tx:
        Sender<super::init_workers::consistency_probe_worker::ConsistencyProbeRequest>,
    pub consistency_probe_rx:
        Receiver<super::init_workers::consistency_probe_worker::ConsistencyProbeResult>,

    // CLIPBOARD (Copy/Cut/Paste)
    pub clipboard: ClipboardManager,

    // CONTEXT MENU STATE
    pub context_menu: ContextMenuState,
    /// Channel to send requests to the shell menu background thread (async extraction).
    pub shell_menu_req_tx:
        std::sync::mpsc::Sender<crate::infrastructure::shell_menu_worker::ShellMenuRequest>,
    /// Channel to receive results from the shell menu background thread.
    pub shell_menu_res_rx:
        std::sync::mpsc::Receiver<crate::infrastructure::shell_menu_worker::ShellMenuResponse>,
    /// True while the background thread is extracting shell items for the active menu.
    pub shell_menu_loading: bool,
    /// Monotonic id used to discard stale async shell-menu responses.
    pub shell_menu_request_id: u64,

    // PERSISTENT ICON LOADER (avoids creating a new one each frame)
    pub item_icon_loader: IconLoader,

    // OPTIMIZED GIF MANAGER
    pub gif_manager: crate::ui::components::gif_manager::GifManager,

    // ASYNC ICON WORKER (avoids blocking I/O in the render loop)
    pub icon_req_sender: Sender<(PathBuf, usize)>, // UI â†’ Worker
    pub icon_res_receiver: Receiver<(PathBuf, usize, Vec<u8>, u32, u32)>, // Worker â†’ UI
    pub loading_icons: FxHashSet<PathBuf>,         // Tracking in-progress
    pub loading_extensions: rustc_hash::FxHashSet<String>, // Dedup by extension (prevent 10x .dll requests)
    pub failed_icons: LruCache<PathBuf, ()>, // Icons that failed extraction (LRU bounded)

    // NOTIFICATION SYSTEM (toast messages)
    pub notifications: crate::application::NotificationManager,

    /// Pending confirmation for high-risk shell-open sources (UNC/shell namespace).
    /// First attempt warns; second attempt on the same path within a short window confirms.
    pub pending_shell_open_confirmation: Option<(PathBuf, Instant)>,

    // STARTUP OPTIMIZATION: Async Font Loading
    pub font_loader_rx: Option<Receiver<egui::FontDefinitions>>,

    // NAVIGATION / ADDRESS BAR (Breadcrumbs vs Edit)
    pub is_address_editing: bool,
    pub show_address_history_menu: bool,

    // SCROLL TO SELECTED (for keyboard navigation)
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

    // Debounce for Shift+Delete key (GetAsyncKeyState fires multiple frames)
    pub delete_key_debounce: bool,

    // One-shot guard: suppress Enter-to-open in file views after committing address bar input.
    pub suppress_next_enter_open: bool,

    // One-shot: focus the address bar on the next toolbar render.
    pub address_bar_focus_request: bool,

    // Window handle for native shell interactions
    pub native_hwnd: Option<HWND>,

    // 3-stage startup: hidden -> maximize/resize -> reveal
    pub startup_tick: usize,

    // Window/layout persistence and list column widths
    pub layout: LayoutState,

    // TAB SYSTEM
    pub tab_manager: crate::tabs::TabManager,

    // DUAL PANEL (split view)
    pub dual_panel_enabled: bool,
    pub dual_panel_active: ActivePanel,
    pub dual_panel_inactive_state: Option<PanelSnapshot>,
    /// When true, `request_thumbnail_load_internal` submits requests using
    /// the active panel's generation (via `current_generation`) and at
    /// `Prefetch` priority instead of the caller-supplied priority.  Set
    /// during inactive panel rendering so thumbnails are loaded into the
    /// shared `texture_cache` without competing with the active panel's
    /// Interactive-priority requests.
    pub suppress_thumbnail_requests: bool,
    /// Transient render guard set only while drawing the inactive dual panel.
    /// Mouse interactions still work there, but global keyboard navigation
    /// must remain owned by the active panel.
    pub suppress_file_panel_keyboard: bool,

    // FOLDER SIZE CALCULATOR (async for details panel)
    pub folder_size_state: FolderSizeState,

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
    pub last_actual_frame_ms: f32,
    pub fps_avg: f32,
    pub upload_budget_ms: f32,
    pub last_upload_budget_update: Instant,
    pub last_memory_maintenance: Instant,
    pub last_texture_cache_retune: Instant,

    // INACTIVITY RECOVERY: Track when app was restored from minimized state
    // Used to throttle heavy operations (watcher events, thumbnail loads) for a few frames
    // after returning from long inactivity, preventing OneDrive-related freezes
    pub last_restore_time: Instant,
    pub minimized_duration_secs: f64,

    // RESTORE BURST MODE: After prolonged idle/minimize, the OS pages out the GPU
    // working set.  Normal adaptive throttling sees slow frames and reduces uploads
    // to 1-2/frame â€” exactly the wrong response.  Burst mode overrides the throttle
    // for a limited window so textures re-populate within ~2-3 seconds.
    pub restore_burst_until: Option<Instant>,

    // PREFERENCES DEBOUNCE: Instead of writing 20+ SQLite rows immediately on every
    // state change (which blocks the UI thread with disk I/O), we set a dirty flag
    // and flush no more than once per second.
    pub preferences_dirty: bool,
    pub preferences_last_save: Instant,

    // Media player volume â€” session-level (updated on slider/keyboard changes, saved to disk on exit)
    pub session_volume: f32,
    // User-selected theme (Light / Dark)
    pub theme_mode: ThemeMode,

    // GPU backend: active backend name (from adapter info, read-only) and user preference
    pub active_gpu_backend: String,
    pub gpu_backend_preference: String,

    // Renderer (Wgpu vs Glow): user preference. `"wgpu"` (default) loads the
    // full wgpu stack; `"glow"` loads the lighter OpenGL-only renderer for a
    // smaller RAM baseline. Requires restart to switch.
    pub renderer_preference: String,

    // Configurable keyboard shortcuts
    pub shortcuts: ShortcutBindings,
    pub shortcut_editor: ShortcutEditorState,

    // Per-folder locked view preferences
    pub folder_locks: std::collections::HashMap<String, crate::domain::folder_lock::FolderLock>,
    pub current_folder_locked: bool,

    // Quick Access pinned folders (ordered by position)
    pub pinned_folders: Vec<crate::domain::pinned_folder::PinnedFolder>,

    // SIDEBAR FOLDER TREE (hierarchical expand/collapse state)
    pub sidebar_tree: sidebar_tree_state::SidebarTreeState,

    // Explicit scroll request for keyboard navigation
    pub scroll_request: ScrollRequest,

    // GLOBAL SEARCH (via MTT Search Service)
    pub global_search: GlobalSearchState,

    // FILE OPERATION WORKER/TRACKING
    pub file_operation_state: FileOperationState,

    // BULK THUMBNAIL SCAN
    pub bulk_thumbnail_scanning: Arc<AtomicBool>,
    pub bulk_thumbnail_was_scanning: bool,
    pub bulk_thumbnail_total: Arc<AtomicUsize>,
    pub bulk_thumbnail_completed: Arc<AtomicUsize>,
    pub bulk_thumbnail_progress: crate::workers::thumbnail::SharedBulkThumbnailProgress,

    // Media keyboard debounce
    pub last_media_key_press: Instant,

    // INACTIVITY RECOVERY: Track window focus for backgroundâ†’foreground detection
    pub was_focused: bool,
    /// Timestamp when the window lost focus (set on focus-lost transition).
    /// Used to measure actual background duration independently of minimize/restore.
    pub focus_lost_at: Option<Instant>,
}

mod helpers;
pub mod sidebar_tree_state;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollRequest {
    None,
    EnsureVisible(usize),
}
