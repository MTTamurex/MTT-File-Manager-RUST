//! Grid view rendering
//! Follows .cursorrules: single responsibility, < 300 lines

use eframe::egui::{self, Color32, Rect, Sense, Ui};
use std::path::PathBuf;

use crate::domain::file_entry::FileEntry;

/// Pre-allocated buffers for pending operations (PERFORMANCE: avoids per-item allocations)
#[derive(Default)]
pub struct PendingOperations {
    pub thumbnail_loads: Vec<(PathBuf, u32)>,
    pub folder_scans: Vec<PathBuf>,
    pub folder_preview_loads: Vec<PathBuf>,
    pub renames: Vec<usize>,
}

impl PendingOperations {
    pub fn new() -> Self {
        Self {
            thumbnail_loads: Vec::with_capacity(16),
            folder_scans: Vec::with_capacity(16),
            folder_preview_loads: Vec::with_capacity(16),
            renames: Vec::with_capacity(2),
        }
    }
    
    /// Clear all buffers (call before each frame)
    pub fn clear(&mut self) {
        self.thumbnail_loads.clear();
        self.folder_scans.clear();
        self.folder_preview_loads.clear();
        self.renames.clear();
    }
}

/// Context for grid view rendering
pub struct GridViewContext<'a> {
    pub items: &'a [FileEntry],
    pub selected_item: Option<usize>,
    pub selected_file: Option<&'a FileEntry>,
    pub multi_selection: &'a std::collections::HashSet<PathBuf>,
    pub thumbnail_size: f32,
    pub last_grid_cols: usize,
    pub renaming_state: Option<(usize, String)>,
    pub focus_rename: bool,
    pub scroll_to_selected: bool, // Scroll to selected item on keyboard navigation
    pub is_computer_view: bool,
    pub is_recycle_bin_view: bool,
    pub texture_cache: &'a mut lru::LruCache<PathBuf, egui::TextureHandle>,
    pub loading_set: &'a mut std::collections::HashSet<PathBuf>,
    pub scanned_folders: &'a mut std::collections::HashSet<PathBuf>,
    pub folder_icon_texture: Option<&'a egui::TextureHandle>,
    pub computer_icon: Option<&'a egui::TextureHandle>,
    pub drive_icon_cache: &'a mut lru::LruCache<String, egui::TextureHandle>,
    pub item_icon_loader: &'a mut crate::ui::icon_loader::IconLoader,
    pub folder_preview_cache: &'a mut lru::LruCache<PathBuf, egui::TextureHandle>,
    pub folder_preview_loading: &'a mut std::collections::HashSet<PathBuf>,
    /// PERFORMANCE: Shared buffer for pending operations (reused across items)
    pub pending_ops: &'a mut PendingOperations,
    /// Caminhos que falharam no thumbnail
    pub failed_thumbnails: &'a std::collections::HashSet<PathBuf>,
    /// Scroll offset for manual virtualization
    pub scroll_offset_y: f32,
    /// Mutable reference to update scroll offset
    pub mut_scroll_offset_y: &'a mut f32,
    /// Last input type for hover control
    pub last_input: crate::app::state::LastInput,
}

/// Operations that can be performed from grid view
pub trait GridViewOperations {
    fn navigate_to(&mut self, path: &str);
    fn open_with_shell(&mut self, path: &PathBuf);
    fn request_thumbnail_load(&mut self, path: PathBuf, size: u32);
    fn request_folder_scan(&mut self, path: PathBuf);
    fn request_folder_preview_load(&mut self, path: PathBuf);
    fn request_thumbnail_prefetch(&mut self, path: PathBuf, size: u32);
    fn rename_with_shell(&mut self, idx: usize);
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
    let mut empty_area_secondary_click = false;
    let mut visible_rows_range = None;

    let available_rect = ui.available_rect_before_wrap();

    /// Helper to render a single grid item with full interaction
    fn render_grid_item(
        ui: &mut Ui,
        index: usize,
        item: &FileEntry,
        rect: Rect,
        ctx: &mut GridViewContext,
        clicked_item: &mut Option<usize>,
        double_clicked_item: &mut Option<usize>,
        secondary_clicked_item: &mut Option<usize>,
    ) {
        let response = ui.interact(rect, ui.id().with(index), Sense::click());
        if response.clicked() {
            *clicked_item = Some(index);
        }
        if response.double_clicked() {
            *double_clicked_item = Some(index);
        }
        if response.secondary_clicked() {
            *secondary_clicked_item = Some(index);
        }

        // --- VISUAL FEEDBACK: BORDER-ONLY (MODERN DESIGN) ---
        let is_selected = ctx.multi_selection.contains(&item.path);
        
        // STRICT HOVER LOGIC: Only allow hover if LastInput was Mouse
        let allow_hover = matches!(ctx.last_input, crate::app::state::LastInput::Mouse);
        let is_hovered_visual = allow_hover && response.hovered() && !is_selected;
        
        let is_focused = ctx.selected_item == Some(index);

        let rounding = 4.0;
        let accent_color = crate::ui::theme::COLOR_ACCENT;

        if is_selected {
            // Selected: Bold primary border
            let stroke_width = if is_hovered_visual { 2.5 } else { 2.0 };
            ui.painter().rect_stroke(
                rect,
                rounding,
                egui::Stroke::new(stroke_width, accent_color),
                egui::StrokeKind::Inside,
            );
        } else if is_hovered_visual || is_focused {
            // Hovered or Focused: Thin subtle border
            let hover_color = accent_color.gamma_multiply(0.35); // ~35% alpha
            ui.painter().rect_stroke(
                rect,
                rounding,
                egui::Stroke::new(1.0, hover_color),
                egui::StrokeKind::Inside,
            );
        }

        if response.hovered() {
            let is_recycle = ctx.is_recycle_bin_view;
            let mouse_pos = ui.input(|i| i.pointer.hover_pos()).unwrap_or_default();
            // SMART TOOLTIP: Inverte se estiver perto da borda direita
            let right_bound = ui.ctx().screen_rect().right();
            let tooltip_pos = if mouse_pos.x + 320.0 > right_bound {
                mouse_pos - egui::vec2(320.0, 0.0)
            } else {
                mouse_pos
            };

            egui::show_tooltip_at(
                ui.ctx(),
                ui.layer_id(),
                response.id,
                tooltip_pos,
                |ui: &mut Ui| {
                    ui.set_max_width(300.0);
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new(&item.name).strong());
                        ui.separator();
                        ui.label(format!("Tipo: {}", get_file_type_string(item)));
                        if !item.is_dir {
                            ui.label(format!(
                                "Tamanho: {}",
                                crate::infrastructure::windows::format_size(item.size)
                            ));
                        }
                        let (date_lbl, date_val) = if is_recycle {
                            ("Data de Exclusão", item.deletion_date.clone().unwrap_or_else(|| "-".to_string()))
                        } else {
                            ("Última modificação", crate::infrastructure::windows::format_date(item.modified))
                        };
                        ui.label(format!("{}: {}", date_lbl, date_val));
                    });
                },
            );
        }

        let inner_rect = rect.shrink(3.0);
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(inner_rect), |ui| {
            render_item_slot_for_grid(ui, index, item, ctx);
        });
    }

    // --- MANUAL VIRTUALIZATION START ---
    let cell_h = item_h + padding;
    let total_rows = (count as f32 / cols as f32).ceil() as usize;
    let total_content_height = total_rows as f32 * cell_h + padding;

    // Viewport area
    let viewport_rect = ui.available_rect_before_wrap();
    let viewport_h = viewport_rect.height();

    // 1. Handle mouse wheel scroll (Manual Source of Truth)
    let scroll_delta = ui.input(|i| i.smooth_scroll_delta.y);
    if scroll_delta != 0.0 {
        // Multiplier for speed as requested
        *ctx.mut_scroll_offset_y -= scroll_delta * 5.0; 
    }

    // 2. Clamp scroll offset
    let max_scroll = (total_content_height - viewport_h).max(0.0);
    *ctx.mut_scroll_offset_y = ctx.mut_scroll_offset_y.clamp(0.0, max_scroll);
    let current_scroll = *ctx.mut_scroll_offset_y;

    // 3. Render Virtual Grid
    ui.allocate_rect(viewport_rect, Sense::hover());
    let mut child_ui = ui.child_ui(viewport_rect, *ui.layout(), None);
    child_ui.set_clip_rect(viewport_rect);

    let content_min = viewport_rect.min;

    if ctx.is_computer_view {
        // Computer view still uses sections, but we can simplify or keep it linear for now
        // Given the requirement "manual scroll manual + viewport + render seletivo"
        // Let's implement Computer View as a special case within the scrollable area
        
        let mut current_y = content_min.y - current_scroll;
        
        let mut local = Vec::new();
        let mut network = Vec::new();
        for (i, item) in ctx.items.iter().enumerate() {
            let is_remote = item.drive_info.as_ref().map_or(false, |di| {
                di.drive_type == crate::infrastructure::windows::DriveType::Remote
            });
            if is_remote { network.push((i, item)); } else { local.push((i, item)); }
        }

        let mut render_section = |ui: &mut Ui, title: &str, items: Vec<(usize, &FileEntry)>, start_y: &mut f32| {
            if items.is_empty() { return; }
            
            // Header
            let header_h = 25.0;
            let header_rect = Rect::from_min_size(egui::pos2(content_min.x, *start_y), egui::vec2(available_w, header_h));
            if ui.is_rect_visible(header_rect) {
                let mut header_ui = ui.child_ui(header_rect, *ui.layout(), None);
                render_section_header(&mut header_ui, title);
            }
            *start_y += header_h;

            let count = items.len();
            let rows = (count as f32 / cols as f32).ceil() as usize;
            let section_h = rows as f32 * cell_h + padding;

            for (i, (index, item)) in items.into_iter().enumerate() {
                let row = i / cols;
                let col = i % cols;
                let x_pos = col as f32 * (item_w + padding) + padding;
                let y_pos = row as f32 * cell_h + padding;
                
                let item_rect = Rect::from_min_size(
                    egui::pos2(content_min.x + x_pos, *start_y + y_pos),
                    egui::vec2(item_w, item_h),
                );

                if ui.is_rect_visible(item_rect) {
                    render_grid_item(ui, index, item, item_rect, ctx, &mut clicked_item, &mut double_clicked_item, &mut secondary_clicked_item);
                }
            }
            *start_y += section_h;
        };

        render_section(&mut child_ui, "Discos locais", local, &mut current_y);
        render_section(&mut child_ui, "Unidades de rede", network, &mut current_y);

    } else {
        // Regular Grid Virtualization
        let vis_min_row = (current_scroll / cell_h).floor() as usize;
        let vis_max_row = ((current_scroll + viewport_h) / cell_h).ceil() as usize;
        
        // Export range for prefetch
        visible_rows_range = Some((vis_min_row, vis_max_row));

        // Overscan
        let loop_min_row = vis_min_row.saturating_sub(1);
        let loop_max_row = (vis_max_row + 1).min(total_rows);

        for row in loop_min_row..loop_max_row {
            for col in 0..cols {
                let index = row * cols + col;
                if index >= count { break; }

                let x_pos = col as f32 * (item_w + padding) + padding;
                let y_pos = row as f32 * cell_h + padding - current_scroll;
                let item_rect = Rect::from_min_size(
                    content_min + egui::vec2(x_pos, y_pos),
                    egui::vec2(item_w, item_h),
                );

                if child_ui.is_rect_visible(item_rect) {
                    render_grid_item(&mut child_ui, index, &ctx.items[index], item_rect, ctx, &mut clicked_item, &mut double_clicked_item, &mut secondary_clicked_item);
                }
            }
        }
    }

    // 4. Custom Scrollbar
    if total_content_height > viewport_h {
        let scrollbar_w = 12.0;
        let scrollbar_rect = Rect::from_min_max(
            viewport_rect.right_top() - egui::vec2(scrollbar_w, 0.0),
            viewport_rect.right_bottom()
        );
        
        // Background
        ui.painter().rect_filled(scrollbar_rect, 0.0, Color32::from_gray(245));

        // Handle
        let handle_h = (viewport_h / total_content_height * viewport_h).max(30.0);
        let handle_y = (current_scroll / max_scroll) * (viewport_h - handle_h);
        let handle_rect = Rect::from_min_size(
            scrollbar_rect.min + egui::vec2(2.0, handle_y),
            egui::vec2(scrollbar_w - 4.0, handle_h)
        );

        let interact = ui.interact(scrollbar_rect, ui.id().with("scrollbar"), Sense::drag());
        if interact.dragged() {
            let delta_y = interact.drag_delta().y;
            let scroll_pct_delta = delta_y / (viewport_h - handle_h);
            *ctx.mut_scroll_offset_y += scroll_pct_delta * max_scroll;
            *ctx.mut_scroll_offset_y = ctx.mut_scroll_offset_y.clamp(0.0, max_scroll);
        }

        let color = if interact.dragged() { Color32::from_gray(150) } else if interact.hovered() { Color32::from_gray(180) } else { Color32::from_gray(200) };
        ui.painter().rect_filled(handle_rect, 4.0, color);
    }
    // --- MANUAL VIRTUALIZATION END ---



    // Header helper
    fn render_section_header(ui: &mut Ui, title: &str) {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new(title)
                .size(13.0)
                .color(Color32::from_gray(120))
                .strong(),
        );
        ui.add_space(4.0);
    }

    // BATCH PROCESSING: Flush all pending operations collected during render
    // This avoids context switching and virtual dispatch inside the render loop
    for (path, size) in ctx.pending_ops.thumbnail_loads.drain(..) {
        ops.request_thumbnail_load(path, size);
    }
    for path in ctx.pending_ops.folder_scans.drain(..) {
        ops.request_folder_scan(path);
    }
    for path in ctx.pending_ops.folder_preview_loads.drain(..) {
        ops.request_folder_preview_load(path);
    }
    for rename_idx in ctx.pending_ops.renames.drain(..) {
        ops.rename_with_shell(rename_idx);
    }

    // PREFETCH LOGIC (Low Priority)
    if let Some((vis_min, vis_max)) = visible_rows_range {
        let count = ctx.items.len();
        let rows = (count as f32 / cols as f32).ceil() as usize;
        let prefetch_margin = 7; // Approx 1 page of cache
        
        let start_prefetch = vis_min.saturating_sub(prefetch_margin);
        let end_prefetch = (vis_max + prefetch_margin).min(rows);
        
        for row in start_prefetch..end_prefetch {
            // Skip visible rows (already handled by render loop)
            // Buffer of 2 was used in render loop, so we stick to that to avoid overlap overkill
            // although overlap is harmless due to loading_set check
            if row >= vis_min.saturating_sub(2) && row < (vis_max + 2).min(rows) {
                continue;
            }

            for col in 0..cols {
                let index = row * cols + col;
                if index >= count { break; }
                
                let item = &ctx.items[index];
                if !item.is_dir {
                    // Check if needs thumbnail
                    if !ctx.texture_cache.contains(&item.path) && !ctx.loading_set.contains(&item.path) {
                        ctx.loading_set.insert(item.path.clone());
                        ops.request_thumbnail_prefetch(item.path.clone(), ctx.thumbnail_size as u32);
                    }
                }
            }
        }
    }

    // Handle actions after rendering - ORDER MATTERS!
    // double_clicked and secondary_clicked must be checked BEFORE clicked
    // because clicked() also returns true on double-click
    if let Some(idx) = double_clicked_item {
        return Some(GridViewAction::DoubleClick(idx));
    }

    if let Some(idx) = secondary_clicked_item {
        return Some(GridViewAction::SecondaryClick(idx));
    }

    // Fallback global: detect secondary click on empty area if no item was clicked
    if secondary_clicked_item.is_none() && ui.input(|i| i.pointer.secondary_clicked()) {
        if let Some(pos) = ui.ctx().pointer_latest_pos() {
            if available_rect.contains(pos) {
                empty_area_secondary_click = true;
            }
        }
    }

    if empty_area_secondary_click {
        return Some(GridViewAction::EmptyAreaSecondaryClick);
    }

    if let Some(idx) = clicked_item {
        return Some(GridViewAction::Click(idx));
    }

    None
}

/// Renders an individual item slot for grid view
/// PERFORMANCE: Uses shared buffers from ctx.pending_ops instead of allocating per-item
fn render_item_slot_for_grid(
    ui: &mut Ui,
    idx: usize,
    item: &FileEntry,
    ctx: &mut GridViewContext
) {
    use crate::ui::components::item_slot::{render_item_slot, ItemSlotContext};

    let is_renaming = ctx
        .renaming_state
        .as_ref()
        .map_or(false, |(i, _)| *i == idx);

    // Texto de renomeação precisa ser tratado separadamente
    let mut renaming_text_clone = if is_renaming {
        ctx.renaming_state.as_ref().map(|(_, s)| s.clone())
    } else {
        None
    };

    // Create context with mutable reference to the clone
    {
        let renaming_text = renaming_text_clone.as_mut();

        let mut item_slot_ctx = ItemSlotContext {
            item,
            idx,
            thumbnail_size: ctx.thumbnail_size,
            is_renaming,
            renaming_text,
            focus_rename: ctx.focus_rename,
            is_recycle_bin_view: ctx.is_recycle_bin_view,
            texture_cache: ctx.texture_cache,
            icon_loader: ctx.item_icon_loader,
            scanned_folders: ctx.scanned_folders,
            loading_set: ctx.loading_set,
            folder_preview_cache: ctx.folder_preview_cache,
            folder_preview_loading: ctx.folder_preview_loading,
            failed_thumbnails: ctx.failed_thumbnails,
        };

        // PERFORMANCE: SimpleOps now writes directly to shared buffers
        struct SimpleOps<'a> {
            pending_ops: &'a mut PendingOperations,
        }

        impl<'a> crate::ui::components::item_slot::ItemSlotOperations for SimpleOps<'a> {
            fn request_thumbnail_load(&mut self, path: std::path::PathBuf, size: u32) {
                self.pending_ops.thumbnail_loads.push((path, size));
            }

            fn request_folder_scan(&mut self, path: std::path::PathBuf) {
                self.pending_ops.folder_scans.push(path);
            }
            fn request_folder_preview_load(&mut self, path: std::path::PathBuf) {
                self.pending_ops.folder_preview_loads.push(path);
            }

            fn rename_item(&mut self, idx: usize) {
                self.pending_ops.renames.push(idx);
            }
        }

        let mut simple_ops = SimpleOps {
            pending_ops: ctx.pending_ops,
        };

        render_item_slot(ui, &mut item_slot_ctx, &mut simple_ops);
    }

    // Apply changes after render
    if let Some(new_text) = renaming_text_clone {
        if is_renaming {
            if let Some((_, ref mut text)) = ctx.renaming_state {
                *text = new_text;
            }
        }
    }
}

/// Helper function to get file type string
fn get_file_type_string(item: &FileEntry) -> String {
    // Check for ZIP manually because is_dir might be true
    if item.name.to_lowercase().ends_with(".zip") {
        return "Arquivo ZIP".to_string();
    }
    if item.is_dir {
        return "Pasta".to_string();
    }
    if let Some(ext) = item.path.extension() {
        return format!("Arquivo {}", ext.to_string_lossy().to_uppercase());
    }
    "Arquivo".to_string()
}
