//! Grid view rendering
//! Follows .cursorrules: single responsibility, < 300 lines

use eframe::egui::{self, Sense, Ui};
use std::path::{Path, PathBuf};

use crate::domain::file_entry::FileEntry;
// PERFORMANCE: Use FxHashSet for PathBuf keys - faster hashing than std::collections::HashSet
use crate::ui::cache::FxHashSet;
mod interactions;
mod item_renderer;
mod prefetch;
mod scroll;
mod virtualization;

// PERFORMANCE: Tooltip debounce to avoid creation/destruction during scroll
const TOOLTIP_DELAY_SECS: f32 = 0.3; // Only show tooltip after 300ms hover
                                     // STRICT LIMIT: Mínimo zoom permitido para evitar degradação de performance
const MIN_THUMBNAIL_SIZE: f32 = 96.0;

#[derive(Clone, Copy)]
pub struct ScrollPredictor {
    last_visible_start: usize,
    last_visible_end: usize,
    scroll_direction: ScrollDirection,
    velocity: f32,
}

#[derive(Clone, Copy, PartialEq)]
pub enum ScrollDirection {
    None,
    Down,
    Up,
}

impl ScrollPredictor {
    pub fn new() -> Self {
        Self {
            last_visible_start: 0,
            last_visible_end: 0,
            scroll_direction: ScrollDirection::None,
            velocity: 0.0,
        }
    }

    pub fn update(&mut self, visible_start: usize, visible_end: usize) {
        if visible_start > self.last_visible_start {
            self.scroll_direction = ScrollDirection::Down;
            self.velocity = (visible_start - self.last_visible_start) as f32;
        } else if visible_start < self.last_visible_start {
            self.scroll_direction = ScrollDirection::Up;
            self.velocity = (self.last_visible_start - visible_start) as f32;
        } else {
            self.velocity *= 0.9;
            if self.velocity < 0.5 {
                self.scroll_direction = ScrollDirection::None;
            }
        }

        self.last_visible_start = visible_start;
        self.last_visible_end = visible_end;
    }

    pub fn get_prefetch_range(&self, total_items: usize) -> (usize, usize) {
        let prefetch_count = 20;
        match self.scroll_direction {
            ScrollDirection::Down => {
                let start = self.last_visible_end;
                let end = (start + prefetch_count).min(total_items);
                (start, end)
            }
            ScrollDirection::Up => {
                let end = self.last_visible_start;
                let start = end.saturating_sub(prefetch_count);
                (start, end)
            }
            ScrollDirection::None => {
                let mid = (self.last_visible_start + self.last_visible_end) / 2;
                let start = mid.saturating_sub(prefetch_count / 2);
                let end = (mid + prefetch_count / 2).min(total_items);
                (start, end)
            }
        }
    }
}

impl Default for ScrollPredictor {
    fn default() -> Self {
        Self::new()
    }
}

/// Pre-allocated buffers for pending operations (PERFORMANCE: avoids per-item allocations)
#[derive(Default)]
pub struct PendingOperations {
    pub thumbnail_loads: Vec<(PathBuf, u32, Option<usize>, u64)>,
    pub folder_scans: Vec<PathBuf>,
    pub folder_preview_loads: Vec<PathBuf>,
    pub icon_loads: Vec<PathBuf>,
    pub renames: Vec<usize>,
}

impl PendingOperations {
    pub fn new() -> Self {
        Self {
            thumbnail_loads: Vec::with_capacity(16),
            folder_scans: Vec::with_capacity(16),
            folder_preview_loads: Vec::with_capacity(16),
            icon_loads: Vec::with_capacity(16),
            renames: Vec::with_capacity(2),
        }
    }

    /// Clear all buffers (call before each frame)
    pub fn clear(&mut self) {
        self.thumbnail_loads.clear();
        self.folder_scans.clear();
        self.folder_preview_loads.clear();
        self.icon_loads.clear();
        self.renames.clear();
    }
}

/// Context for grid view rendering
pub struct GridViewContext<'a> {
    pub items: &'a [FileEntry],
    pub selected_item: Option<usize>,
    pub selected_file: Option<&'a FileEntry>,
    pub multi_selection: &'a FxHashSet<PathBuf>,
    pub thumbnail_size: f32,
    pub last_grid_cols: usize,
    pub renaming_state: Option<(usize, String)>,
    pub focus_rename: bool,
    pub scroll_to_selected: bool, // Scroll to selected item on keyboard navigation
    pub is_computer_view: bool,
    pub is_recycle_bin_view: bool,
    pub global_search_active: bool,
    pub texture_cache: &'a mut lru::LruCache<PathBuf, egui::TextureHandle>,
    pub loading_set: &'a mut FxHashSet<PathBuf>,
    /// Set of icons currently loading (async)
    pub loading_icons: &'a mut FxHashSet<PathBuf>,
    /// Set of icons that failed extraction (prevents infinite retry)
    pub failed_icons: &'a lru::LruCache<PathBuf, ()>,
    pub scanned_folders: &'a mut lru::LruCache<PathBuf, ()>,
    pub folder_icon_texture: Option<&'a egui::TextureHandle>,
    pub computer_icon: Option<&'a egui::TextureHandle>,
    pub drive_icon_cache: &'a mut lru::LruCache<String, egui::TextureHandle>,
    pub item_icon_loader: &'a mut crate::ui::icon_loader::IconLoader,
    pub folder_preview_cache: &'a mut lru::LruCache<PathBuf, egui::TextureHandle>,
    pub folder_preview_loading: &'a mut FxHashSet<PathBuf>,
    /// PERFORMANCE: Shared buffer for pending operations (reused across items)
    pub pending_ops: &'a mut PendingOperations,
    /// Caminhos que falharam no thumbnail (LRU bounded)
    pub failed_thumbnails: &'a lru::LruCache<PathBuf, ()>,
    /// Scroll offset for manual virtualization
    pub scroll_offset_y: f32,
    /// Mutable reference to update scroll offset
    pub mut_scroll_offset_y: &'a mut f32,
    pub last_input: crate::app::state::LastInput,
    pub scroll_predictor: &'a mut ScrollPredictor,
    /// PERFORMANCE: Scroll state tracking for GPU upload throttling
    pub last_scroll_time: &'a mut std::time::Instant,
    pub last_scroll_offset: &'a mut f32,
    /// Conjunto de itens aguardando upload GPU
    pub pending_upload_set: &'a mut FxHashSet<PathBuf>,
    pub is_video_docked_visible: bool,
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
    /// PERFORMANCE: Pre-computed local drive indices for computer view (avoids per-frame allocation)
    pub computer_local_indices: &'a [usize],
    /// PERFORMANCE: Pre-computed network drive indices for computer view
    pub computer_network_indices: &'a [usize],
}

/// Operations that can be performed from grid view
pub trait GridViewOperations {
    fn navigate_to(&mut self, path: &str);
    fn open_with_shell(&mut self, path: &Path);
    fn request_thumbnail_load(&mut self, path: PathBuf, size: u32, modified: u64);
    fn request_thumbnail_load_with_index(
        &mut self,
        path: PathBuf,
        size: u32,
        directory_index: usize,
        modified: u64,
    );
    fn request_folder_scan(&mut self, path: PathBuf);
    fn request_folder_preview_load(&mut self, path: PathBuf);
    fn request_thumbnail_prefetch(&mut self, path: PathBuf, size: u32, modified: u64);
    fn request_thumbnail_prefetch_with_index(
        &mut self,
        path: PathBuf,
        size: u32,
        directory_index: usize,
        modified: u64,
    );
    fn request_icon_load(&mut self, path: PathBuf);
    fn rename_with_shell(&mut self, idx: usize);
    fn notify_idle_visible_items(&mut self, items: Vec<PathBuf>);
}

/// Action returned by grid view
pub enum GridViewAction {
    Click(usize),
    DoubleClick(usize),
    SecondaryClick(usize),
    EmptyAreaSecondaryClick,
}

/// Renders the grid view
pub fn render_grid_view(
    ui: &mut Ui,
    ctx: &mut GridViewContext,
    ops: &mut dyn GridViewOperations,
) -> Option<GridViewAction> {
    // ENFORCE MINIMUM ZOOM (Hard Floor)
    // Impede qualquer cálculo ou render com tamanho menor que 96px
    ctx.thumbnail_size = ctx.thumbnail_size.max(MIN_THUMBNAIL_SIZE);

    let padding = 8.0;
    let item_w = ctx.thumbnail_size;
    let item_h = ctx.thumbnail_size + 20.0; // Height: thumb + text
    let available_w = ui.available_width();
    let cols = ((available_w - padding) / (item_w + padding))
        .floor()
        .max(1.0) as usize;
    ctx.last_grid_cols = cols;

    // Keyboard navigation (handled by caller)

    let count = ctx.items.len();
    // Virtualized grid or Grouped grid
    let mut clicked_item = None;
    let mut double_clicked_item = None;
    let mut secondary_clicked_item = None;
    #[allow(unused_assignments)]
    let mut visible_rows_range: Option<(usize, usize)> = None;
    // --- MANUAL VIRTUALIZATION START ---
    let visual_cell_h = item_h + padding;
    const MIN_VIRTUAL_CELL_HEIGHT: f32 = 24.0;
    let virtual_cell_h = visual_cell_h.max(MIN_VIRTUAL_CELL_HEIGHT);

    let total_rows = (count as f32 / cols as f32).ceil() as usize;
    let total_content_height = total_rows as f32 * virtual_cell_h + padding;

    // Viewport area
    let viewport_rect = ui.available_rect_before_wrap();
    let viewport_h = viewport_rect.height();
    let max_scroll = (total_content_height - viewport_h).max(0.0);
    let pointer_over_viewport = ui
        .ctx()
        .pointer_hover_pos()
        .is_some_and(|pos| viewport_rect.contains(pos));
    let consume_scroll = pointer_over_viewport && !ctx.global_search_active;

    scroll::apply_scroll_input(ui, ctx.mut_scroll_offset_y, max_scroll, consume_scroll);
    let (current_scroll, scroll_delta) =
        scroll::compute_visual_scroll(ui, *ctx.mut_scroll_offset_y, viewport_h);

    // PERFORMANCE: Track scroll changes
    if (*ctx.mut_scroll_offset_y - *ctx.last_scroll_offset).abs() > 0.1 {
        *ctx.last_scroll_time = std::time::Instant::now();
        *ctx.last_scroll_offset = *ctx.mut_scroll_offset_y;
    }
    // Is scrolling if visual position is changing (using same threshold)
    let is_scrolling = scroll_delta > 0.5;

    // 2.5 KEYBOARD SCROLL SYNC: Ensure selected item is visible
    if ctx.scroll_to_selected {
        if let Some(selected_idx) = ctx.selected_item {
            if selected_idx < count {
                let selected_row = selected_idx / cols;
                let item_top = selected_row as f32 * virtual_cell_h + padding;
                let item_bottom = item_top + item_h; // Keep item_h for visual bottom check

                // We check against TARGET scroll to ensure we snap to the final correct position
                // but we might want to check visual if we want to smooth scroll TO the item.
                // For now, snap target instantly as per requirement (keyboard nav usually snaps)
                let current_target = *ctx.mut_scroll_offset_y;

                if item_top < current_target {
                    *ctx.mut_scroll_offset_y = item_top.max(0.0);
                } else if item_bottom > current_target + viewport_h {
                    *ctx.mut_scroll_offset_y = (item_bottom - viewport_h).clamp(0.0, max_scroll);
                }
            }
        }
    }

    // 3. Render Virtual Grid
    // DETECT BACKGROUND INTERACTION
    let bg_response = ui.interact(viewport_rect, ui.id().with("grid_bg"), Sense::click());
    visible_rows_range = virtualization::render_virtualized_grid(
        ui,
        ctx,
        viewport_rect,
        viewport_h,
        current_scroll,
        total_rows,
        count,
        cols,
        padding,
        item_w,
        item_h,
        available_w,
        virtual_cell_h,
        is_scrolling,
        &mut clicked_item,
        &mut double_clicked_item,
        &mut secondary_clicked_item,
    );

    scroll::render_custom_scrollbar(
        ui,
        viewport_rect,
        viewport_h,
        total_content_height,
        current_scroll,
        max_scroll,
        ctx.mut_scroll_offset_y,
    );
    // --- MANUAL VIRTUALIZATION END ---

    prefetch::flush_pending_operations(ctx, ops);
    prefetch::process_visible_range_prefetch(ctx, cols, visible_rows_range, ops);

    interactions::resolve_grid_action(
        clicked_item,
        double_clicked_item,
        secondary_clicked_item,
        bg_response.secondary_clicked(),
    )
}
