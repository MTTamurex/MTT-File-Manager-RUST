//! Grid view rendering
//! Follows .cursorrules: single responsibility, < 300 lines

use eframe::egui::{self, Color32, Rect, Sense, Ui};
use std::path::PathBuf;

use crate::domain::file_entry::FileEntry;

// PERFORMANCE: Tooltip debounce to avoid creation/destruction during scroll
const TOOLTIP_DELAY_SECS: f32 = 0.3; // Only show tooltip after 300ms hover
// STRICT LIMIT: Mínimo zoom permitido para evitar degradação de performance
const MIN_THUMBNAIL_SIZE: f32 = 96.0;

/// Scroll state tracking for visual smoothing
#[derive(Clone, Copy, Debug)]
struct ScrollState {
    visual_scroll_y: f32,
}

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
    /// Caminhos que falharam no thumbnail (LRU bounded)
    pub failed_thumbnails: &'a lru::LruCache<PathBuf, ()>,
    /// Scroll offset for manual virtualization
    pub scroll_offset_y: f32,
    /// Mutable reference to update scroll offset
    pub mut_scroll_offset_y: &'a mut f32,
    pub last_input: crate::app::state::LastInput,
    /// PERFORMANCE: Scroll state tracking for GPU upload throttling
    pub last_scroll_time: &'a mut std::time::Instant,
    pub last_scroll_offset: &'a mut f32,
    /// Conjunto de itens aguardando upload GPU
    pub pending_upload_set: &'a mut std::collections::HashSet<PathBuf>,
    /// PERFORMANCE: True if video is playing in docked mode (reduces prefetch to minimize HDD I/O)
    pub is_video_playing_docked: bool,
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
    let mut empty_area_secondary_click = false;
    #[allow(unused_assignments)]
    let mut visible_rows_range: Option<(usize, usize)> = None;

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

        // PERFORMANCE: Tooltip with debounce to avoid spam during scroll
        if response.hovered() {
                let current_time = ui.input(|i| i.time);
                let hover_id = response.id.with("hover_start");

                // Track hover start time using egui's memory
                let hover_start_time = ui.ctx().data_mut(|d| {
                    *d.get_temp_mut_or_insert_with(hover_id, || current_time)
                });

                let hover_duration = (current_time - hover_start_time) as f32;

                // Request repaint when approaching tooltip delay to ensure it appears
                if hover_duration < TOOLTIP_DELAY_SECS {
                    ui.ctx().request_repaint_after(std::time::Duration::from_secs_f32(
                        TOOLTIP_DELAY_SECS - hover_duration + 0.01
                    ));
                }

                // Only show tooltip if hover duration exceeds threshold
                // This prevents tooltip spam during scroll
                if hover_duration >= TOOLTIP_DELAY_SECS {
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
                                let is_zip = item.name.to_lowercase().ends_with(".zip");
                                if !item.is_dir || is_zip {
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
            } else {
                // Clear hover time when not hovering
                let hover_id = response.id.with("hover_start");
                ui.ctx().data_mut(|d| d.remove::<f64>(hover_id));
            }

        // STANDARD RENDERING
        let inner_rect = rect.shrink(3.0);
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(inner_rect), |ui| {
            render_item_slot_for_grid(ui, index, item, ctx);
        });
    }

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

    // 1. Handle Input (Target Scroll)
    let scroll_delta = ui.input(|i| i.smooth_scroll_delta.y);
    if scroll_delta != 0.0 {
        // SCROLL CONFIGURATION - PIXEL BASED
        let notches = scroll_delta / 50.0;
        let px_per_notch = ctx.thumbnail_size + padding; // Height + Padding (Simplificado conforme regra)
        
        let move_px = notches * px_per_notch;
        
        *ctx.mut_scroll_offset_y -= move_px;
    }

    // 1.5 Clamp Target
    *ctx.mut_scroll_offset_y = ctx.mut_scroll_offset_y.clamp(0.0, max_scroll);
    
    // 2. Interpolate Visual Scroll (Frame-based smoothing)
    let scroll_target = *ctx.mut_scroll_offset_y;
    let scroll_state_id = ui.id().with("scroll_state");
    // Limit dt to avoid massive jumps on lag spikes (e.g., 30ms max)
    let dt = ui.input(|i| i.stable_dt).min(0.03); 
    
    let visual_scroll = ui.ctx().data_mut(|d| {
        let state = d.get_temp_mut_or_insert_with::<ScrollState>(scroll_state_id, || ScrollState {
            visual_scroll_y: scroll_target,
        });

        // LERP: Move 25% of the way to target per 16ms frame (approx)
        // Adjust '15.0' to tune stiffness/smoothness
        let t = (dt * 15.0).min(1.0);
        state.visual_scroll_y = state.visual_scroll_y + (scroll_target - state.visual_scroll_y) * t;

        // Snap to target if very close to stop micro-adjustments
        if (state.visual_scroll_y - scroll_target).abs() < 0.5 {
            state.visual_scroll_y = scroll_target;
        }

        state.visual_scroll_y
    });

    // Request repaint if we are still animating towards target
    if visual_scroll != scroll_target {
        ui.ctx().request_repaint();
    }

    // Use visual_scroll for rendering from here on
    let current_scroll = visual_scroll;

    // PERFORMANCE: Track scroll changes for GPU upload throttling (using visual scroll to capture checking)
    // Note: We update last_scroll_offset matching target to keep logic consistent with state, 
    // but we use visual change to trigger "is moving" logic.
    if (*ctx.mut_scroll_offset_y - *ctx.last_scroll_offset).abs() > 0.1 {
        *ctx.last_scroll_time = std::time::Instant::now();
        *ctx.last_scroll_offset = *ctx.mut_scroll_offset_y;
    }
    // Simple "is scrolling" check for optimization (if visual is changing, we are scrolling)
    let is_scrolling = visual_scroll != scroll_target;

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
    
    let mut child_ui = ui.new_child(egui::UiBuilder::new().max_rect(viewport_rect));
    child_ui.set_clip_rect(viewport_rect);

    let content_min = viewport_rect.min;

    // Virtualization Math (using Interpolated Visual Scroll)
    let vis_min_row = (current_scroll / virtual_cell_h).floor() as usize;
    let vis_max_row = ((current_scroll + viewport_h) / virtual_cell_h).ceil() as usize;

    // Export range for prefetch relative to visual position
    visible_rows_range = Some((vis_min_row, vis_max_row));

    // PERFORMANCE: Clear stale loading_set entries when scrolling
    // This ensures that slots are freed for currently visible items.
    // Without this, the loading_set fills with items from previous scroll positions
    // and new visible items can't load (blocked by the loading limit).
    // Only clean if loading_set has significant entries to avoid overhead on every frame.
    if ctx.loading_set.len() > 30 {
        // Build set of paths that SHOULD remain (visible range + generous margin)
        let cleanup_margin = 8; // Keep items within 8 rows of visible area
        let keep_min_row = vis_min_row.saturating_sub(cleanup_margin);
        let keep_max_row = (vis_max_row + cleanup_margin).min(total_rows);

        // Collect paths to keep based on row index
        let mut paths_to_keep = std::collections::HashSet::with_capacity((keep_max_row - keep_min_row) * cols);
        for row in keep_min_row..keep_max_row {
            for col in 0..cols {
                let index = row * cols + col;
                if index < count {
                    paths_to_keep.insert(ctx.items[index].path.clone());
                    // Also keep folder covers
                    if let Some(ref cover) = ctx.items[index].folder_cover {
                        paths_to_keep.insert(cover.clone());
                    }
                }
            }
        }

        // Remove stale entries (paths not in visible range + margin)
        ctx.loading_set.retain(|path| paths_to_keep.contains(path));
    }

    // STABLE OVERSCAN: Fixed value, no velocity dependency
    let overscan = 2;

    let pre_clamp_min_row = vis_min_row.saturating_sub(overscan);
    let pre_clamp_max_row = (vis_max_row + overscan).min(total_rows);

    // Standard Virtualization Limits (with overscan)
    let loop_min_row = pre_clamp_min_row;
    let loop_max_row = pre_clamp_max_row;

    if ctx.is_computer_view {
        // Computer view with sections (Manual Scroll & Layout)
        let mut current_y = content_min.y - current_scroll;
        
        // ZERO-ALLOCATION RENDERING: Iterate directly instead of creating vectors
        
        // Helper to render a section by filtering items on the fly
        // We pass a closure to filter: is_network_drive -> bool
        let mut render_section_direct = |ui: &mut Ui, title: &str, is_network: bool, start_y: &mut f32| {
            // first count items to calculate height (cheap iteration)
            let mut section_count = 0;
            for item in ctx.items.iter() {
                let is_remote = item.drive_info.as_ref().map_or(false, |di| {
                    di.drive_type == crate::infrastructure::windows::DriveType::Remote
                });
                if is_remote == is_network {
                    section_count += 1;
                }
            }

            if section_count == 0 { return; }

            // Header
            let header_h = 25.0;
            // Check visibility of header
            if *start_y + header_h > content_min.y && *start_y < content_min.y + viewport_h {
                let header_rect = Rect::from_min_size(egui::pos2(content_min.x, *start_y), egui::vec2(available_w, header_h));
                let mut header_ui = ui.new_child(egui::UiBuilder::new().max_rect(header_rect));
                render_section_header(&mut header_ui, title);
            }
            *start_y += header_h;

            let rows = (section_count as f32 / cols as f32).ceil() as usize;
            let section_h = rows as f32 * virtual_cell_h + padding;

            // Render items in this section
            // Optimization: Only iterate if section is visible
            if *start_y + section_h > content_min.y && *start_y < content_min.y + viewport_h {
                let mut current_idx = 0;
                for (real_idx, item) in ctx.items.iter().enumerate() {
                    let is_remote = item.drive_info.as_ref().map_or(false, |di| {
                        di.drive_type == crate::infrastructure::windows::DriveType::Remote
                    });
                    
                    if is_remote == is_network {
                        // Calculate position
                        let row = current_idx / cols;
                        let col_idx = current_idx % cols;
                        
                        let item_y = *start_y + row as f32 * virtual_cell_h + padding;
                        
                        // Culling check
                        if item_y + item_h > content_min.y && item_y < content_min.y + viewport_h {
                             let x_pos = col_idx as f32 * (item_w + padding) + padding;
                             let item_rect = Rect::from_min_size(
                                egui::pos2(content_min.x + x_pos, item_y),
                                egui::vec2(item_w, item_h),
                            );
                            render_grid_item(ui, real_idx, item, item_rect, ctx, &mut clicked_item, &mut double_clicked_item, &mut secondary_clicked_item);
                        }
                        
                        current_idx += 1;
                    }
                }
            }

            *start_y += section_h;
        };

        render_section_direct(&mut child_ui, "Discos locais", false, &mut current_y);
        render_section_direct(&mut child_ui, "Unidades de rede", true, &mut current_y);

    } else {
        // Standard Grid Virtualization
        for row in loop_min_row..loop_max_row {
            for col in 0..cols {
                let index = row * cols + col;
                if index >= count { break; }

                let x_pos = col as f32 * (item_w + padding) + padding;
                let y_pos = content_min.y + row as f32 * virtual_cell_h + padding - current_scroll;
                
                let item_rect = Rect::from_min_size(
                    egui::pos2(content_min.x + x_pos, y_pos),
                    egui::vec2(item_w, item_h),
                );

                render_grid_item(&mut child_ui, index, &ctx.items[index], item_rect, ctx, &mut clicked_item, &mut double_clicked_item, &mut secondary_clicked_item);
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
        
        ui.painter().rect_filled(scrollbar_rect, 0.0, Color32::from_gray(245));

        let handle_h = (viewport_h / total_content_height * viewport_h).max(30.0);
        // Use VISUAL scroll for handle position to match rendering
        let handle_y = (current_scroll / max_scroll) * (viewport_h - handle_h);
        let handle_rect = Rect::from_min_size(
            scrollbar_rect.min + egui::vec2(2.0, handle_y),
            egui::vec2(scrollbar_w - 4.0, handle_h)
        );

        let interact = ui.interact(scrollbar_rect, ui.id().with("scrollbar"), Sense::click_and_drag());
        
        if interact.clicked() {
            if let Some(click_pos) = ui.input(|i| i.pointer.interact_pos()) {
                let relative_y = click_pos.y - scrollbar_rect.top();
                let target_handle_top = relative_y - (handle_h / 2.0);
                let scroll_ratio = target_handle_top / (viewport_h - handle_h);
                // Update TARGET
                *ctx.mut_scroll_offset_y = (scroll_ratio * max_scroll).clamp(0.0, max_scroll);
            }
        } else if interact.dragged() {
            let delta_y = interact.drag_delta().y;
            let scroll_pct_delta = delta_y / (viewport_h - handle_h);
            // Update TARGET
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
    // Note: Thumbnail cache is on SSD, so we don't skip I/O even during video playback
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

    // PERFORMANCE: Adaptive prefetch based on scroll state
    // Note: Thumbnail cache is on SSD, so prefetch is always beneficial
    if let Some((vis_min, vis_max)) = visible_rows_range {
        let count = ctx.items.len();
        let rows = (count as f32 / cols as f32).ceil() as usize;

        // Adaptive prefetch margin based on scroll status
        let prefetch_margin = if is_scrolling {
            2 // During scroll, moderate prefetch
        } else {
            5 // When idle, prefetch more ahead
        };

        let start_prefetch = vis_min.saturating_sub(prefetch_margin);
        let end_prefetch = (vis_max + prefetch_margin).min(rows);

        for row in start_prefetch..end_prefetch {
            // Skip visible + overscan rows (already handled by render loop)
            if row >= vis_min.saturating_sub(overscan) && row < (vis_max + overscan).min(rows) {
                continue;
            }

            for col in 0..cols {
                let index = row * cols + col;
                if index >= count { break; }

                let item = &ctx.items[index];
                if !item.is_dir {
                    // Check if needs thumbnail - skip if already in cache, loading, or pending upload
                    if !ctx.texture_cache.contains(&item.path)
                        && !ctx.loading_set.contains(&item.path)
                        && !ctx.pending_upload_set.contains(&item.path)
                    {
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
    if secondary_clicked_item.is_none() && bg_response.secondary_clicked() {
        empty_area_secondary_click = true;
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
            pending_upload_set: ctx.pending_upload_set,
            is_dense_mode: false, // Legacy: dense mode logic removed from grid view
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
