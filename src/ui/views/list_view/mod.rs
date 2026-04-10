//! List view rendering - modular structure
//! Splits rendering into header, virtualization, item rendering, and helpers

mod header;
mod helpers;
mod item_renderer;
mod item_renderer_details;
mod scroll;
mod virtualization;

use eframe::egui::{self, Color32, FontId, Ui};
use lru::LruCache;
use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

use crate::domain::file_entry::{FileEntry, SortMode};
// PERFORMANCE: Use FxHashSet for PathBuf keys - faster hashing than std::collections::HashSet
use crate::ui::cache::FxHashSet;

// PERFORMANCE: Thread-local cache for font metrics using u64 hash key with LRU eviction
// Uses LruCache instead of HashMap to avoid full clear when limit is reached
// Capacity: 5000 entries - least recently used entries are evicted automatically
thread_local! {
    static FONT_WIDTH_CACHE: RefCell<LruCache<u64, f32>> = RefCell::new(LruCache::new(
        NonZeroUsize::new(5000).expect("font width cache size must be non-zero")
    ));
}

// PERFORMANCE: Thread-local cache for text truncation results to avoid redundant binary search
// Key is hash of (text, max_width as bits), value is the truncated string
// M-6: LruCache for truncation results — evicts LRU entry on overflow instead of wiping all 2000
thread_local! {
    static TRUNCATION_CACHE: RefCell<LruCache<u64, String>> = RefCell::new(LruCache::new(
        NonZeroUsize::new(2000).expect("truncation cache size must be non-zero")
    ));
}

/// Clear the truncation cache (call when column widths change)
#[allow(dead_code)]
pub fn clear_truncation_cache() {
    TRUNCATION_CACHE.with(|cache| {
        cache.borrow_mut().clear();
    });
}

/// Compute hash key for (text, max_width) pair
#[inline]
fn truncation_cache_key(text: &str, max_width: f32) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    max_width.to_bits().hash(&mut hasher);
    hasher.finish()
}


/// Compute hash key for (text, font_size) pair
/// PERFORMANCE: Uses precomputed hash to avoid String allocation for cache key
#[inline]
fn font_width_cache_key(text: &str, font_size: u32) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.as_bytes().hash(&mut hasher);
    font_size.hash(&mut hasher);
    hasher.finish()
}

/// Get text width from cache or compute and cache it
/// PERFORMANCE: Uses u64 hash key instead of (String, u32, Color32) to avoid String allocation
fn get_cached_text_width(text: &str, font_id: &FontId, _color: Color32, ui: &Ui) -> f32 {
    let key = font_width_cache_key(text, font_id.size as u32);

    FONT_WIDTH_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();

        if let Some(&width) = cache.get(&key) {
            return width;
        }

        // PERFORMANCE: Use provided Color32 for layout (API requirement), but don't include in key
        // since text width is independent of color
        let width = ui.fonts(|f| {
            f.layout_no_wrap(text.to_string(), font_id.clone(), _color)
                .rect
                .width()
        });
        cache.put(key, width);
        width
    })
}

/// Helper to truncate text to fit within a column width
/// PERFORMANCE: Uses byte-position slicing instead of chars().take().collect() to avoid
/// String allocations on each binary search iteration. Only one String is created at the end.
/// Also uses font width cache to avoid redundant text layout calculations.
/// Cache: Uses thread-local cache keyed by (text, max_width) to avoid recomputation.
pub(crate) fn truncate_text_for_column(
    text: &str,
    max_width: f32,
    font_id: &FontId,
    ui: &Ui,
) -> String {
    // PERFORMANCE: Check cache first using hash of (text, max_width)
    let cache_key = truncation_cache_key(text, max_width);
    let cached_result = TRUNCATION_CACHE.with(|cache| cache.borrow_mut().get(&cache_key).cloned());
    if let Some(result) = cached_result {
        return result;
    }

    // Quick check: does full text fit?
    let full_width = get_cached_text_width(text, font_id, Color32::WHITE, ui);
    if full_width <= max_width {
        let result = text.to_string();
        // Cache the result
        TRUNCATION_CACHE.with(|cache| cache.borrow_mut().put(cache_key, result.clone()));
        return result;
    }

    let ellipsis = "...";
    let ellipsis_width = get_cached_text_width(ellipsis, font_id, Color32::WHITE, ui);
    let available_width = max_width - ellipsis_width;

    if available_width <= 0.0 {
        let result = ellipsis.to_string();
        // Cache the result
        TRUNCATION_CACHE.with(|cache| cache.borrow_mut().put(cache_key, result.clone()));
        return result;
    }

    // Build char boundary table once (byte positions of each char boundary)
    let char_boundaries: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
    let char_count = char_boundaries.len();

    if char_count == 0 {
        let result = ellipsis.to_string();
        // Cache the result
        TRUNCATION_CACHE.with(|cache| cache.borrow_mut().put(cache_key, result.clone()));
        return result;
    }

    // Binary search on char index, using &str slices (no allocation per iteration)
    let mut left = 0usize;
    let mut right = char_count;

    while left < right {
        let mid = (left + right).div_ceil(2);
        let byte_end = if mid < char_count {
            char_boundaries[mid]
        } else {
            text.len()
        };
        let slice = &text[..byte_end];
        let w = get_cached_text_width(slice, font_id, Color32::WHITE, ui);

        if w <= available_width {
            left = mid;
        } else {
            right = mid - 1;
        }
    }

    if left == 0 {
        let result = ellipsis.to_string();
        // Cache the result
        TRUNCATION_CACHE.with(|cache| cache.borrow_mut().put(cache_key, result.clone()));
        return result;
    }

    let byte_end = if left < char_count {
        char_boundaries[left]
    } else {
        text.len()
    };
    let mut result = String::with_capacity(byte_end + 3);
    result.push_str(&text[..byte_end]);
    result.push_str(ellipsis);

    // Cache the result before returning
    TRUNCATION_CACHE.with(|cache| cache.borrow_mut().put(cache_key, result.clone()));

    result
}

/// Column widths snapshot for item rendering
pub(crate) struct ColumnWidths {
    pub name: f32,
    pub date: f32,
    pub type_col: f32,
    pub size: f32,
}

/// Context for list view rendering
pub struct ListViewContext<'a> {
    pub items: &'a [FileEntry],
    pub selected_item: Option<usize>,
    pub selected_file: Option<&'a FileEntry>,
    pub multi_selection: &'a FxHashSet<PathBuf>,
    pub sort_mode: SortMode,
    pub sort_descending: bool,
    pub renaming_state: Option<(usize, String)>,
    pub focus_rename: bool,
    pub scroll_to_selected: bool, // Scroll to selected item on keyboard navigation
    pub generation: usize,
    pub is_computer_view: bool,
    pub is_recycle_bin_view: bool,
    pub is_onedrive_folder: bool,
    pub global_search_active: bool,
    pub texture_cache: &'a mut lru::LruCache<PathBuf, egui::TextureHandle>,
    pub loading_set: &'a mut FxHashSet<PathBuf>,
    pub loading_icons: &'a mut FxHashSet<PathBuf>,
    /// Set of icons that failed extraction (prevents infinite retry)
    pub failed_icons: &'a lru::LruCache<PathBuf, ()>,
    pub scanned_folders: &'a mut lru::LruCache<PathBuf, ()>,
    pub folder_icon_texture: Option<&'a egui::TextureHandle>,
    pub computer_icon: Option<&'a egui::TextureHandle>,
    pub drive_icon_cache: &'a mut lru::LruCache<String, egui::TextureHandle>,
    pub item_icon_loader: &'a mut crate::ui::icon_loader::IconLoader,
    pub deletion_date_cache: Option<&'a mut lru::LruCache<String, String>>, // Cache for deletion dates (Path string -> Date)
    /// Paths that failed thumbnail generation (LRU bounded)
    pub failed_thumbnails: &'a lru::LruCache<PathBuf, ()>,
    /// Scroll offset for manual virtualization
    pub scroll_offset_y: f32,
    /// Mutable reference to update scroll offset
    pub mut_scroll_offset_y: &'a mut f32,
    pub last_input: crate::app::state::LastInput,
    /// PERFORMANCE: Scroll state tracking for GPU upload throttling
    pub last_scroll_time: &'a mut std::time::Instant,
    pub last_scroll_offset: &'a mut f32,
    /// Set of items awaiting GPU upload
    pub pending_upload_set: &'a mut FxHashSet<PathBuf>,
    pub is_video_docked_visible: bool,
    /// PERFORMANCE: True when current path is on HDD (not SSD)
    pub is_on_hdd: bool,
    pub prefetch_rows: usize,
    /// Output: visible item index range for GPU upload prioritization
    pub visible_index_range: &'a mut Option<(usize, usize)>,
    /// Whether an item drag operation is active
    pub is_item_dragging: bool,
    /// Current folder path under drop target highlight
    pub drag_target_folder: Option<PathBuf>,
    /// Output: item where drag started this frame
    pub drag_started_item: &'a mut Option<usize>,
    /// Output: currently hovered folder item during drag
    pub drag_hovered_item: &'a mut Option<usize>,
    pub live_file_size_cache: &'a mut lru::LruCache<PathBuf, (u64, u64)>,
    pub live_file_size_loading: &'a mut FxHashSet<PathBuf>,
    pub live_file_size_req_sender:
        &'a std::sync::mpsc::Sender<crate::app::live_file_size::LiveFileSizeRequest>,
    // Resizable column widths
    pub col_name_width: &'a mut f32,
    pub col_date_width: &'a mut f32,
    pub col_type_width: &'a mut f32,
    pub col_size_width: &'a mut f32,
    pub col_status_width: &'a mut f32, // OneDrive only
}

/// Action returned by list view
pub enum ListViewAction {
    Click(usize),
    DoubleClick(usize),
    SecondaryClick(usize),
    SortChange(SortMode),
    EmptyAreaSecondaryClick,
}

/// Operations that can be performed from list view
pub trait ListViewOperations {
    fn navigate_to(&mut self, path: &str);
    fn open_with_shell(&mut self, path: &Path);
    fn request_thumbnail_load(&mut self, path: PathBuf, directory_index: usize, modified: u64);
    fn request_folder_scan(&mut self, path: PathBuf);
    fn request_folder_preview_load(&mut self, path: PathBuf);
    fn rename_with_shell(&mut self, idx: usize);
    fn request_thumbnail_prefetch_with_index(
        &mut self,
        path: PathBuf,
        size: u32,
        directory_index: usize,
        modified: u64,
    );
    fn request_icon_load(&mut self, path: PathBuf);
    fn notify_idle_visible_items(&mut self, items: Vec<PathBuf>);
}

/// Renders the list view
pub fn render_list_view(
    ui: &mut Ui,
    ctx: &mut ListViewContext,
    ops: &mut dyn ListViewOperations,
) -> Option<ListViewAction> {
    let row_height = 24.0;
    let available_w = ui.available_width();

    // Snapshot column widths BEFORE scaling (used as scaling input)
    let w_status = if ctx.is_onedrive_folder && !ctx.is_computer_view {
        *ctx.col_status_width
    } else {
        0.0
    };
    let w_name = *ctx.col_name_width;
    let w_date = *ctx.col_date_width;
    let w_type = *ctx.col_type_width;
    let w_size = *ctx.col_size_width;

    // Scale column widths if they exceed available space
    scale_column_widths(ctx, available_w, w_status, w_name, w_date, w_type, w_size);

    // Snapshot for item rendering (uses post-scaling values to match header)
    let col_widths = ColumnWidths {
        name: *ctx.col_name_width,
        date: *ctx.col_date_width,
        type_col: *ctx.col_type_width,
        size: *ctx.col_size_width,
    };

    // Render header (uses ctx.col_*_width directly for resize interaction)
    let sort_action = header::render_list_header(ui, ctx, available_w);

    ui.separator();

    // Render virtualized content
    let interaction = virtualization::render_virtualized_content(
        ui,
        ctx,
        ops,
        available_w,
        row_height,
        &col_widths,
    );

    // Handle actions after rendering - ORDER MATTERS!
    if interaction.empty_area_secondary_click {
        return Some(ListViewAction::EmptyAreaSecondaryClick);
    }

    // Sort header clicks take priority
    if let Some(mode) = sort_action {
        return Some(ListViewAction::SortChange(mode));
    }

    // double_clicked and secondary_clicked must be checked BEFORE clicked
    // because clicked() also returns true on double-click
    if let Some(idx) = interaction.double_clicked_item {
        return Some(ListViewAction::DoubleClick(idx));
    }

    if let Some(idx) = interaction.secondary_clicked_item {
        return Some(ListViewAction::SecondaryClick(idx));
    }

    if let Some(idx) = interaction.clicked_item {
        return Some(ListViewAction::Click(idx));
    }

    None
}

/// Scales column widths proportionally if they exceed available space
fn scale_column_widths(
    ctx: &mut ListViewContext,
    available_w: f32,
    w_status: f32,
    w_name: f32,
    w_date: f32,
    w_type: f32,
    w_size: f32,
) {
    // Ensure total column width doesn't exceed available space
    // Reserve 8px for scrollbar
    let max_total_width = available_w - 8.0;
    if max_total_width <= 0.0 {
        return;
    }

    // Calculate total based on which columns are actually visible
    let current_total = if ctx.is_computer_view {
        // Computer View: Name + Date (as "Total Space") + Size (as "Free Space")
        w_name + w_date + w_size
    } else if ctx.is_onedrive_folder {
        // OneDrive View: Name + Date + Type + Size + Status
        w_name + w_date + w_type + w_size + w_status
    } else {
        // Regular View: Name + Date + Type + Size
        w_name + w_date + w_type + w_size
    };

    if current_total > max_total_width {
        // Proportionally reduce visible columns to fit
        let scale = max_total_width / current_total;
        *ctx.col_name_width = (w_name * scale).max(100.0);
        *ctx.col_date_width = (w_date * scale).max(80.0);
        if ctx.is_computer_view {
            *ctx.col_size_width = (w_size * scale).max(80.0);
        } else if ctx.is_onedrive_folder {
            *ctx.col_type_width = (w_type * scale).max(80.0);
            *ctx.col_size_width = (w_size * scale).max(80.0);
            *ctx.col_status_width = (w_status * scale).max(80.0);
        } else {
            *ctx.col_type_width = (w_type * scale).max(80.0);
            *ctx.col_size_width = (w_size * scale).max(80.0);
        }
    }
}
