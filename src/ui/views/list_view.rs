//! List view rendering
//! Follows .cursorrules: single responsibility, < 300 lines

use eframe::egui::{self, Color32, FontId, Pos2, Rect, RichText, Sense, Ui};
use std::path::PathBuf;

use crate::domain::file_entry::{FileEntry, SortMode, SyncStatus};
use crate::infrastructure::windows::{format_date, format_size};
// PERFORMANCE: Use FxHashSet for PathBuf keys - faster hashing than std::collections::HashSet
use crate::ui::cache::FxHashSet;
use crate::ui::views::ViewportTracker;

// PERFORMANCE: Tooltip debounce to avoid creation/destruction during scroll
const TOOLTIP_DELAY_SECS: f32 = 0.3; // Only show tooltip after 300ms hover

/// Helper to truncate text to fit within a column width
fn truncate_text_for_column(text: &str, max_width: f32, font_id: &FontId, ui: &Ui) -> String {
    let fonts = ui.fonts(|f| f.clone());
    let galley = fonts.layout_no_wrap(text.to_string(), font_id.clone(), Color32::WHITE);
    
    if galley.rect.width() <= max_width {
        return text.to_string();
    }
    
    // Binary search for optimal length
    let ellipsis = "...";
    let ellipsis_galley = fonts.layout_no_wrap(ellipsis.to_string(), font_id.clone(), Color32::WHITE);
    let ellipsis_width = ellipsis_galley.rect.width();
    let available_width = max_width - ellipsis_width;
    
    if available_width <= 0.0 {
        return ellipsis.to_string();
    }
    
    let mut left = 0;
    let mut right = text.chars().count();
    
    while left < right {
        let mid = (left + right + 1) / 2;
        let truncated: String = text.chars().take(mid).collect();
        let test_galley = fonts.layout_no_wrap(truncated.clone(), font_id.clone(), Color32::WHITE);
        
        if test_galley.rect.width() <= available_width {
            left = mid;
        } else {
            right = mid - 1;
        }
    }
    
    if left == 0 {
        return ellipsis.to_string();
    }
    
    let truncated: String = text.chars().take(left).collect();
    format!("{}{}", truncated, ellipsis)
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
    pub is_computer_view: bool,
    pub is_recycle_bin_view: bool,
    pub is_onedrive_folder: bool,
    pub texture_cache: &'a mut lru::LruCache<PathBuf, egui::TextureHandle>,
    pub loading_set: &'a mut FxHashSet<PathBuf>,
    pub loading_icons: &'a mut FxHashSet<PathBuf>,
    /// Set of icons that failed extraction (prevents infinite retry)
    pub failed_icons: &'a FxHashSet<PathBuf>,
    pub scanned_folders: &'a mut FxHashSet<PathBuf>,
    pub folder_icon_texture: Option<&'a egui::TextureHandle>,
    pub computer_icon: Option<&'a egui::TextureHandle>,
    pub drive_icon_cache: &'a mut lru::LruCache<String, egui::TextureHandle>,
    pub item_icon_loader: &'a mut crate::ui::icon_loader::IconLoader,
    pub deletion_date_cache: Option<&'a mut lru::LruCache<String, String>>, // Cache para datas de exclusão (Path string -> Data)
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
    pub pending_upload_set: &'a mut FxHashSet<PathBuf>,
    pub is_video_docked_visible: bool,
    pub prefetch_rows: usize,
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
    fn open_with_shell(&mut self, path: &PathBuf);
    fn request_thumbnail_load(&mut self, path: PathBuf, directory_index: usize);
    fn request_folder_scan(&mut self, path: PathBuf);
    fn request_folder_preview_load(&mut self, path: PathBuf);
    fn rename_with_shell(&mut self, idx: usize);
    fn request_thumbnail_prefetch_with_index(
        &mut self,
        path: PathBuf,
        size: u32,
        directory_index: usize,
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

    // Use status column width from context when in OneDrive folder
    let w_status = if ctx.is_onedrive_folder && !ctx.is_computer_view {
        *ctx.col_status_width
    } else {
        0.0
    };
    
    // Ensure total column width doesn't exceed available space
    // Reserve 8px for scrollbar
    let max_total_width = available_w - 8.0;
    
    // Use mutable column widths from context
    let w_name = *ctx.col_name_width;
    let w_date = *ctx.col_date_width;
    let w_type = *ctx.col_type_width;
    let w_size = *ctx.col_size_width;
    
    // Calculate total based on which columns are actually visible
    let current_total = if ctx.is_computer_view {
        // Computer View: Name + Date (as "Espaço Total") + Size (as "Espaço Livre")
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

    // Table header - capture sort mode change
    let mut sort_action: Option<SortMode> = None;

    ui.horizontal(|ui| {
        ui.style_mut().spacing.item_spacing.x = 0.0;

        // Calculate available space for columns (total - scrollbar - status column)
        let available_for_columns = available_w - 8.0 - w_status;

        // Draw header with resize handle
        let draw_header_resizable = |ui: &mut Ui, text: &str, width: &mut f32, mode: SortMode, min_width: f32, other_widths: f32| {
            let header_rect = egui::Rect::from_min_size(
                ui.cursor().min,
                egui::vec2(*width, 22.0)
            );
            
            // Header clickable area (for sorting)
            let header_id = ui.id().with(format!("header_{}", text));
            let header_response = ui.interact(header_rect, header_id, Sense::click());
            
            let is_active = ctx.sort_mode == mode;

            if ui.is_rect_visible(header_rect) {
                if is_active {
                    ui.painter().rect_filled(header_rect, 2.0, Color32::from_gray(230));
                }
                let text_color = if is_active {
                    Color32::BLACK
                } else {
                    Color32::from_gray(100)
                };
                
                // Truncate text to fit within column
                let available_text_width = *width - 30.0; // Reserve space for arrow and padding
                let font_id = FontId::proportional(12.0);
                let full_text_galley = ui.fonts(|f| f.layout_no_wrap(text.to_string(), font_id.clone(), text_color));
                
                let display_text = if full_text_galley.rect.width() > available_text_width {
                    // Truncate with ellipsis
                    let mut truncated = text.to_string();
                    while !truncated.is_empty() {
                        let test_text = format!("{}...", truncated);
                        let test_galley = ui.fonts(|f| f.layout_no_wrap(test_text.clone(), font_id.clone(), text_color));
                        if test_galley.rect.width() <= available_text_width {
                            break;
                        }
                        truncated.pop();
                    }
                    if truncated.is_empty() {
                        "...".to_string()
                    } else {
                        format!("{}...", truncated)
                    }
                } else {
                    text.to_string()
                };
                
                ui.painter().text(
                    header_rect.min + egui::vec2(8.0, 4.0),
                    egui::Align2::LEFT_TOP,
                    display_text,
                    font_id,
                    text_color,
                );
                
                if is_active {
                    let arrow = if ctx.sort_descending { "▼" } else { "▲" };
                    ui.painter().text(
                        header_rect.max - egui::vec2(15.0, 8.0),
                        egui::Align2::CENTER_CENTER,
                        arrow,
                        FontId::proportional(10.0),
                        text_color,
                    );
                }
            }
            
            // Resize handle (right edge of column)
            let handle_width = 8.0;
            let handle_rect = egui::Rect::from_min_size(
                egui::pos2(header_rect.max.x - handle_width / 2.0, header_rect.min.y),
                egui::vec2(handle_width, 22.0)
            );
            
            let handle_id = ui.id().with(format!("resize_{}", text));
            let handle_response = ui.interact(handle_rect, handle_id, Sense::click_and_drag());
            
            // Change cursor on hover
            if handle_response.hovered() || handle_response.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }
            
            // Handle resize drag with max constraint
            if handle_response.dragged() {
                let delta = handle_response.drag_delta().x;
                let max_width = available_for_columns - other_widths;
                // Prevent panic: ensure max_width is never less than min_width
                if max_width >= min_width {
                    *width = (*width + delta).clamp(min_width, max_width);
                } else {
                    // If there's not enough space, just enforce min_width
                    *width = min_width;
                }
            }
            
            // Draw resize handle indicator on hover
            if handle_response.hovered() || handle_response.dragged() {
                ui.painter().rect_filled(
                    handle_rect.shrink2(egui::vec2(2.0, 4.0)),
                    0.0,
                    Color32::from_rgb(100, 150, 200)
                );
            }
            
            // Advance cursor
            ui.allocate_exact_size(egui::vec2(*width, 22.0), Sense::hover());
            
            header_response.clicked()
        };

        // Calculate current widths for constraint checks
        let current_date = *ctx.col_date_width;
        let current_size = *ctx.col_size_width;

        if draw_header_resizable(ui, "Nome", ctx.col_name_width, SortMode::Name, 100.0, current_date + current_size) {
            return Some(SortMode::Name);
        }

        if ctx.is_computer_view {
            // Computer View: apenas Nome, Espaço Total e Espaço Livre (sem Tipo)
            // Recalculate after potential Name resize
            let current_name = *ctx.col_name_width;
            let current_size = *ctx.col_size_width;
            
            if draw_header_resizable(ui, "Espaço Total", ctx.col_date_width, SortMode::DriveTotalSpace, 80.0, current_name + current_size) {
                return Some(SortMode::DriveTotalSpace);
            }

            // Recalculate after potential Date resize
            let current_name = *ctx.col_name_width;
            let current_date = *ctx.col_date_width;
            
            if draw_header_resizable(ui, "Espaço Livre", ctx.col_size_width, SortMode::DriveFreeSpace, 80.0, current_name + current_date) {
                return Some(SortMode::DriveFreeSpace);
            }
        } else {
            // Regular view: Nome, Data, Tipo, Tamanho (+ Status se OneDrive)
            // Recalculate after potential Name resize
            let current_name = *ctx.col_name_width;
            let current_type = *ctx.col_type_width;
            let current_size = *ctx.col_size_width;
            
            let date_label = if ctx.is_recycle_bin_view {
                "Data de Exclusão"
            } else {
                "Última modificação"
            };
            if draw_header_resizable(ui, date_label, ctx.col_date_width, SortMode::Date, 120.0, current_name + current_type + current_size) {
                return Some(SortMode::Date);
            }

            // Recalculate after potential Date resize
            let current_name = *ctx.col_name_width;
            let current_date = *ctx.col_date_width;
            let current_size = *ctx.col_size_width;
            
            if draw_header_resizable(ui, "Tipo", ctx.col_type_width, SortMode::Type, 80.0, current_name + current_date + current_size) {
                return Some(SortMode::Type);
            }

            // Recalculate after potential Type resize
            let current_name = *ctx.col_name_width;
            let current_date = *ctx.col_date_width;
            let current_type = *ctx.col_type_width;
            
            if draw_header_resizable(ui, "Tamanho", ctx.col_size_width, SortMode::Size, 80.0, current_name + current_date + current_type) {
                return Some(SortMode::Size);
            }

            // Status column (OneDrive only) - now resizable
            if ctx.is_onedrive_folder {
                let current_name = *ctx.col_name_width;
                let current_date = *ctx.col_date_width;
                let current_type = *ctx.col_type_width;
                let current_size = *ctx.col_size_width;
                
                // Draw status header with resize capability (no sorting)
                let header_rect = egui::Rect::from_min_size(
                    ui.cursor().min,
                    egui::vec2(*ctx.col_status_width, 22.0)
                );
                
                let header_id = ui.id().with("header_status");
                let _header_response = ui.interact(header_rect, header_id, Sense::hover());
                
                if ui.is_rect_visible(header_rect) {
                    ui.painter().text(
                        header_rect.min + egui::vec2(8.0, 4.0),
                        egui::Align2::LEFT_TOP,
                        "Status",
                        FontId::proportional(12.0),
                        Color32::from_gray(100),
                    );
                }
                
                // Resize handle for Status column
                let handle_width = 8.0;
                let handle_rect = egui::Rect::from_min_size(
                    egui::pos2(header_rect.max.x - handle_width / 2.0, header_rect.min.y),
                    egui::vec2(handle_width, 22.0)
                );
                
                let handle_id = ui.id().with("resize_status");
                let handle_response = ui.interact(handle_rect, handle_id, Sense::click_and_drag());
                
                if handle_response.hovered() || handle_response.dragged() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                }
                
                if handle_response.dragged() {
                    let delta = handle_response.drag_delta().x;
                    let available_for_columns = available_w - 8.0;
                    let other_widths = current_name + current_date + current_type + current_size;
                    let max_width = available_for_columns - other_widths;
                    let min_width = 80.0;
                    
                    if max_width >= min_width {
                        *ctx.col_status_width = (*ctx.col_status_width + delta).clamp(min_width, max_width);
                    } else {
                        *ctx.col_status_width = min_width;
                    }
                }
                
                if handle_response.hovered() || handle_response.dragged() {
                    ui.painter().rect_filled(
                        handle_rect.shrink2(egui::vec2(2.0, 4.0)),
                        0.0,
                        Color32::from_rgb(100, 150, 200)
                    );
                }
                
                ui.allocate_exact_size(egui::vec2(*ctx.col_status_width, 22.0), Sense::hover());
            }
        }

        None
    })
    .inner
    .map(|mode| sort_action = Some(mode));

    ui.separator();
    let available_rect = ui.available_rect_before_wrap();

    let total_rows = ctx.items.len();
    // Virtualized list or Grouped list for Computer View
    let mut clicked_item = None;
    let mut double_clicked_item = None;
    let mut secondary_clicked_item = None;
    let mut empty_area_secondary_click = false;

    // --- MANUAL VIRTUALIZATION START ---
    let total_content_height = total_rows as f32 * row_height;
    let viewport_rect = ui.available_rect_before_wrap();
    let viewport_h = viewport_rect.height();

    // 1. Handle mouse wheel scroll (Manual Source of Truth)
    let scroll_delta = ui.input(|i| i.smooth_scroll_delta.y);
    if scroll_delta != 0.0 {
        *ctx.mut_scroll_offset_y -= scroll_delta * 5.0;
    }

    // 2. Clamp scroll offset
    let max_scroll = (total_content_height - viewport_h).max(0.0);
    *ctx.mut_scroll_offset_y = ctx.mut_scroll_offset_y.clamp(0.0, max_scroll);

    // 2.5 KEYBOARD SCROLL SYNC: Ensure selected item is visible
    if ctx.scroll_to_selected {
        if let Some(selected_idx) = ctx.selected_item {
            if selected_idx < total_rows {
                let item_top = selected_idx as f32 * row_height;
                let item_bottom = item_top + row_height;

                let current_scroll_check = *ctx.mut_scroll_offset_y;

                // Scroll up if item is above viewport
                if item_top < current_scroll_check {
                    *ctx.mut_scroll_offset_y = item_top.max(0.0);
                }
                // Scroll down if item is below viewport
                else if item_bottom > current_scroll_check + viewport_h {
                    *ctx.mut_scroll_offset_y = (item_bottom - viewport_h).clamp(0.0, max_scroll);
                }
            }
        }
    }

    let current_scroll = *ctx.mut_scroll_offset_y;

    // PERFORMANCE: Track scroll changes for GPU upload throttling
    if (current_scroll - *ctx.last_scroll_offset).abs() > 0.1 {
        *ctx.last_scroll_time = std::time::Instant::now();
        *ctx.last_scroll_offset = current_scroll;
    }

    // 3. Render Virtual List
    // DETECT BACKGROUND INTERACTION (Sense::click() captures secondary_clicked without global leakage)
    let bg_response = ui.interact(viewport_rect, ui.id().with("list_bg"), Sense::click());

    let mut child_ui = ui.new_child(egui::UiBuilder::new().max_rect(viewport_rect));
    child_ui.set_clip_rect(viewport_rect);

    let content_min = viewport_rect.min;

    if ctx.is_computer_view {
        // Grouped view for "Este Computador" (Manual Scroll)
        let mut local = Vec::new();
        let mut network = Vec::new();

        for (i, item) in ctx.items.iter().enumerate() {
            let is_remote = item.drive_info.as_ref().is_some_and(|di| {
                di.drive_type == crate::infrastructure::windows::DriveType::Remote
            });
            if is_remote {
                network.push((i, item));
            } else {
                local.push((i, item));
            }
        }

        let mut current_y = content_min.y - current_scroll;

        if !local.is_empty() {
            let header_h = 30.0;
            let header_rect = Rect::from_min_size(
                egui::pos2(content_min.x, current_y),
                egui::vec2(available_w, header_h),
            );
            if child_ui.is_rect_visible(header_rect) {
                let mut header_ui =
                    child_ui.new_child(egui::UiBuilder::new().max_rect(header_rect));
                render_section_header(&mut header_ui, "Discos locais");
            }
            current_y += header_h;

            for (i, item) in local {
                let item_rect = Rect::from_min_size(
                    egui::pos2(content_min.x, current_y),
                    egui::vec2(available_w, row_height),
                );
                if child_ui.is_rect_visible(item_rect) {
                    render_list_item(
                        &mut child_ui,
                        i,
                        item,
                        item_rect,
                        ctx,
                        ops,
                        available_rect,
                        &mut clicked_item,
                        &mut double_clicked_item,
                        &mut secondary_clicked_item,
                        w_name,
                        w_date,
                        w_type,
                        w_size,
                        w_status,
                        row_height,
                    );
                }
                current_y += row_height;
            }
            current_y += 10.0;
        }

        if !network.is_empty() {
            let header_h = 30.0;
            let header_rect = Rect::from_min_size(
                egui::pos2(content_min.x, current_y),
                egui::vec2(available_w, header_h),
            );
            if child_ui.is_rect_visible(header_rect) {
                let mut header_ui =
                    child_ui.new_child(egui::UiBuilder::new().max_rect(header_rect));
                render_section_header(&mut header_ui, "Unidades de rede");
            }
            current_y += header_h;

            for (i, item) in network {
                let item_rect = Rect::from_min_size(
                    egui::pos2(content_min.x, current_y),
                    egui::vec2(available_w, row_height),
                );
                if child_ui.is_rect_visible(item_rect) {
                    render_list_item(
                        &mut child_ui,
                        i,
                        item,
                        item_rect,
                        ctx,
                        ops,
                        available_rect,
                        &mut clicked_item,
                        &mut double_clicked_item,
                        &mut secondary_clicked_item,
                        w_name,
                        w_date,
                        w_type,
                        w_size,
                        w_status,
                        row_height,
                    );
                }
                current_y += row_height;
            }
        }
    } else {
        // Regular virtualized list
        let is_scrolling = std::time::Instant::now()
            .duration_since(*ctx.last_scroll_time)
            .as_millis()
            < 80;
        let overscan = if is_scrolling { 2 } else { 5 };
        let vis_min_row = ((current_scroll / row_height).floor() as usize).saturating_sub(overscan);
        let vis_max_row = (((current_scroll + viewport_h) / row_height).ceil() as usize) + overscan;
        let vis_max_row = vis_max_row.min(total_rows);

        for i in vis_min_row..vis_max_row {
            let item = &ctx.items[i];
            let item_rect = Rect::from_min_size(
                egui::pos2(
                    content_min.x,
                    content_min.y + (i as f32 * row_height) - current_scroll,
                ),
                egui::vec2(available_w, row_height),
            );

            render_list_item(
                &mut child_ui,
                i,
                item,
                item_rect,
                ctx,
                ops,
                available_rect,
                &mut clicked_item,
                &mut double_clicked_item,
                &mut secondary_clicked_item,
                w_name,
                w_date,
                w_type,
                w_size,
                w_status,
                row_height,
            );
        }
    }

    // 4. Custom Scrollbar with Track-Click
    if total_content_height > viewport_h {
        let scroll_bar_w = 4.0;
        let scroll_bar_rect = Rect::from_min_max(
            egui::pos2(
                viewport_rect.right() - scroll_bar_w - 2.0,
                viewport_rect.top(),
            ),
            egui::pos2(viewport_rect.right() - 2.0, viewport_rect.bottom()),
        );

        let handle_h = (viewport_h / total_content_height * viewport_h).max(30.0);
        let handle_top = current_scroll / max_scroll * (viewport_h - handle_h);
        let handle_rect = Rect::from_min_size(
            egui::pos2(scroll_bar_rect.left(), viewport_rect.top() + handle_top),
            egui::vec2(scroll_bar_w, handle_h),
        );

        // Interaction: click_and_drag for both track-click and handle drag
        let scroll_id = ui.id().with("list_scrollbar");
        let response = ui.interact(scroll_bar_rect, scroll_id, Sense::click_and_drag());

        if response.clicked() {
            // TRACK-CLICK: Jump to clicked position
            if let Some(click_pos) = ui.input(|i| i.pointer.interact_pos()) {
                let relative_y = click_pos.y - scroll_bar_rect.top();
                let target_handle_top = relative_y - (handle_h / 2.0);
                let scroll_ratio = target_handle_top / (viewport_h - handle_h);
                *ctx.mut_scroll_offset_y = (scroll_ratio * max_scroll).clamp(0.0, max_scroll);
            }
        } else if response.dragged() {
            let delta = response.drag_delta().y;
            let scroll_per_pixel = max_scroll / (viewport_h - handle_h);
            *ctx.mut_scroll_offset_y += delta * scroll_per_pixel;
            *ctx.mut_scroll_offset_y = ctx.mut_scroll_offset_y.clamp(0.0, max_scroll);
        }

        // Draw track
        ui.painter()
            .rect_filled(scroll_bar_rect, 0.0, Color32::from_black_alpha(10));
        // Draw handle
        let handle_color = if response.dragged() {
            Color32::from_gray(100)
        } else if response.hovered() {
            Color32::from_gray(150)
        } else {
            Color32::from_gray(200)
        };
        ui.painter().rect_filled(handle_rect, 2.0, handle_color);
    }
    // --- MANUAL VIRTUALIZATION END ---

    if total_rows > 0 {
        let first_visible_index = (current_scroll / row_height).floor() as usize;
        let last_visible_index = ((current_scroll + viewport_h) / row_height).ceil() as usize;
        let first_visible_index = first_visible_index.min(total_rows.saturating_sub(1));
        let last_visible_index = last_visible_index.min(total_rows).saturating_sub(1);

        let tracker = ViewportTracker {
            first_visible_index,
            last_visible_index,
            prefetch_rows: ctx.prefetch_rows,
            columns: 1,
        };
        let (prefetch_start, prefetch_end) = tracker.get_prefetch_range(total_rows);

        for index in prefetch_start..prefetch_end {
            if index >= total_rows {
                break;
            }
            if tracker.is_visible(index) {
                continue;
            }
            let item = &ctx.items[index];
            if !item.is_dir {
                if !ctx.texture_cache.contains(&item.path)
                    && !ctx.loading_set.contains(&item.path)
                    && !ctx.pending_upload_set.contains(&item.path)
                {
                    ctx.loading_set.insert(item.path.clone());
                    ops.request_thumbnail_prefetch_with_index(item.path.clone(), 64, index);
                }
            }
        }

        let mut idle_visible_items = Vec::new();
        for index in first_visible_index..=last_visible_index {
            let item = &ctx.items[index];
            if !item.is_dir {
                idle_visible_items.push(item.path.clone());
            }
        }
        if !idle_visible_items.is_empty() {
            ops.notify_idle_visible_items(idle_visible_items);
        }
    }

    // Fallback global: detect secondary click on empty area if no item was clicked
    if secondary_clicked_item.is_none() && bg_response.secondary_clicked() {
        empty_area_secondary_click = true;
    }

    if empty_area_secondary_click {
        return Some(ListViewAction::EmptyAreaSecondaryClick);
    }

    // Handle actions after rendering - ORDER MATTERS!
    // Sort header clicks take priority
    if let Some(mode) = sort_action {
        return Some(ListViewAction::SortChange(mode));
    }

    // double_clicked and secondary_clicked must be checked BEFORE clicked
    // because clicked() also returns true on double-click
    if let Some(idx) = double_clicked_item {
        return Some(ListViewAction::DoubleClick(idx));
    }

    if let Some(idx) = secondary_clicked_item {
        return Some(ListViewAction::SecondaryClick(idx));
    }

    if let Some(idx) = clicked_item {
        return Some(ListViewAction::Click(idx));
    }

    None
}

/// Helper for rendering a single list item
fn render_list_item(
    ui: &mut Ui,
    i: usize,
    item: &FileEntry,
    rect: Rect,
    ctx: &mut ListViewContext,
    ops: &mut dyn ListViewOperations,
    _available_rect: Rect,
    clicked_item: &mut Option<usize>,
    double_clicked_item: &mut Option<usize>,
    secondary_clicked_item: &mut Option<usize>,
    w_name: f32,
    w_date: f32,
    w_type: f32,
    w_size: f32,
    _w_status: f32,
    row_height: f32,
) {
    // GATILHO LAZY LOAD PARA PASTAS: Descobre capa se ainda não tem
    if item.is_dir
        && !ctx.is_computer_view
        && !ctx.is_recycle_bin_view
        && item.folder_cover.is_none()
        && !ctx.scanned_folders.contains(&item.path)
    {
        ctx.scanned_folders.insert(item.path.clone());
        ops.request_folder_scan(item.path.clone());
    }

    // GATILHO LAZY LOAD PARA ARQUIVOS DE MÍDIA: Carrega thumbnail proativamente
    if !item.is_dir && !ctx.is_recycle_bin_view {
        let is_media_file = item
            .path
            .extension()
            .map(|ext| crate::infrastructure::windows::is_media_extension(&ext.to_string_lossy()))
            .unwrap_or(false);

        if is_media_file
            && !ctx.texture_cache.contains(&item.path)
            && !ctx.loading_set.contains(&item.path)
            && !ctx.failed_thumbnails.contains(&item.path)
            && !ctx.pending_upload_set.contains(&item.path)
            && ctx.loading_set.len() < 200
        {
            ctx.loading_set.insert(item.path.clone());
            ops.request_thumbnail_load(item.path.clone(), i);
        }
    }

    let is_recycle_bin = ctx.is_recycle_bin_view;

    ui.push_id(i, |ui| {
        let response = ui.interact(rect, ui.id().with(i), Sense::click());

        // Selection and Action
        if response.clicked() {
            *clicked_item = Some(i);
        }

        if response.double_clicked() {
            *double_clicked_item = Some(i);
        }

        if response.secondary_clicked() {
            *secondary_clicked_item = Some(i);
        }

        // --- VISUAL FEEDBACK: BORDER-ONLY (MODERN DESIGN) ---
        let is_selected = ctx.multi_selection.contains(&item.path);

        // STRICT HOVER LOGIC: Only allow hover if LastInput was Mouse
        let allow_hover = matches!(ctx.last_input, crate::app::state::LastInput::Mouse);
        let is_hovered_visual = allow_hover && response.hovered() && !is_selected;

        let is_focused = ctx.selected_item == Some(i);

        let rounding = 4.0;
        let accent_color = crate::ui::theme::COLOR_ACCENT;

        // ADJUST RECT TO AVOID SCROLLBAR OVERLAP
        // Scrollbar is 4px + 2px margin. Using 8px to ensure a clean gap.
        let mut visual_rect = rect;
        visual_rect.max.x -= 8.0;

        if is_selected {
            // Selected: Bold primary border
            let stroke_width = if is_hovered_visual { 2.5 } else { 2.0 };
            ui.painter().rect_stroke(
                visual_rect,
                rounding,
                egui::Stroke::new(stroke_width, accent_color),
                egui::StrokeKind::Inside,
            );
        } else if is_hovered_visual || is_focused {
            // Hovered or Focused: Thin subtle border
            let hover_color = accent_color.gamma_multiply(0.35); // ~35% alpha as requested
            ui.painter().rect_stroke(
                visual_rect,
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
            let hover_start_time = ui
                .ctx()
                .data_mut(|d| *d.get_temp_mut_or_insert_with(hover_id, || current_time));

            let hover_duration = (current_time - hover_start_time) as f32;

            // Request repaint when approaching tooltip delay to ensure it appears
            if hover_duration < TOOLTIP_DELAY_SECS {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_secs_f32(
                        TOOLTIP_DELAY_SECS - hover_duration + 0.01,
                    ));
            }

            // Only show tooltip if hover duration exceeds threshold
            if hover_duration >= TOOLTIP_DELAY_SECS {
                let mouse_pos = ui.input(|i| i.pointer.hover_pos()).unwrap_or_default();

                // SMART TOOLTIP: Position to avoid video player overlay
                // Native HWND windows (MPV) render above egui content, so we must avoid that area
                let screen_right = ui.ctx().screen_rect().right();
                let tooltip_width = 320.0;

                // When video is docked, the preview panel takes ~25-30% of window width
                // Only flip tooltip when it would actually overlap the video area
                let effective_right = if ctx.is_video_docked_visible {
                    screen_right * 0.72 // Preview panel is ~28% of window
                } else {
                    screen_right
                };

                let tooltip_x = if mouse_pos.x + tooltip_width > effective_right {
                    (effective_right - tooltip_width - 5.0).max(10.0)
                } else {
                    mouse_pos.x
                };
                let tooltip_pos = egui::pos2(tooltip_x, mouse_pos.y);

                // Use Order::Tooltip layer (though it won't help with native HWND windows)
                let tooltip_layer =
                    egui::LayerId::new(egui::Order::Tooltip, response.id.with("tooltip"));
                egui::show_tooltip_at(
                    ui.ctx(),
                    tooltip_layer,
                    response.id,
                    tooltip_pos,
                    |ui: &mut Ui| {
                        ui.set_max_width(300.0);
                        ui.vertical(|ui| {
                            ui.label(RichText::new(&item.name).strong());
                            ui.separator();
                            ui.label(format!("Tipo: {}", get_file_type_string(item)));
                            let is_zip = item.name.to_lowercase().ends_with(".zip");
                            if !item.is_dir || is_zip {
                                ui.label(format!("Tamanho: {}", format_size(item.size)));
                            }
                            let date_lbl = if is_recycle_bin {
                                "Data de Exclusão"
                            } else {
                                "Última modificação"
                            };
                            let date_val = if is_recycle_bin {
                                item.deletion_date
                                    .clone()
                                    .unwrap_or_else(|| "-".to_string())
                            } else {
                                format_date(item.modified)
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

        let text_color = if is_selected {
            crate::ui::theme::COLOR_SELECTION_TEXT
        } else {
            Color32::BLACK
        };
        let secondary_color = if is_selected {
            crate::ui::theme::COLOR_SELECTION_TEXT
        } else {
            Color32::from_gray(100)
        };

        // 1. Icon + Name
        let icon_size_px = 16.0;
        let icon_rect = Rect::from_min_size(
            rect.min + egui::vec2(4.0, 4.0),
            egui::vec2(icon_size_px, icon_size_px),
        );

        if item.drive_info.is_some() {
            // Drive: use specialized drive icon loader
            if let Some(drive_icon) = ctx
                .item_icon_loader
                .get_or_load_drive_icon(ui.ctx(), &item.path.to_string_lossy())
            {
                ui.painter().image(
                    drive_icon.id(),
                    icon_rect,
                    Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            } else {
                ui.painter().text(
                    icon_rect.min,
                    egui::Align2::LEFT_TOP,
                    "💽",
                    FontId::proportional(14.0),
                    Color32::GRAY,
                );
            }
        } else if item.is_dir && !item.name.to_lowercase().ends_with(".zip") {
            // folder: Windows native icon
            let is_virtual_zip = item.path.to_string_lossy().to_lowercase().contains(".zip\\")
                || item.path.to_string_lossy().to_lowercase().contains(".zip/");

            if is_virtual_zip {
                if let Some(folder_icon) = ctx
                    .item_icon_loader
                    .get_or_load_icon(ui.ctx(), &item.path, true, false)
                {
                    ui.painter().image(
                        folder_icon.id(),
                        icon_rect,
                        Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                        Color32::WHITE,
                    );
                } else if let Some(folder_icon) = ctx.folder_icon_texture {
                    ui.painter().image(
                        folder_icon.id(),
                        icon_rect,
                        Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                        Color32::WHITE,
                    );
                } else {
                    ui.painter().text(
                        icon_rect.min,
                        egui::Align2::LEFT_TOP,
                        "\u{ED9F}", // ICON_FOLDER
                        FontId::new(14.0, egui::FontFamily::Name("icons".into())),
                        Color32::from_rgb(255, 193, 7),
                    );
                }
            } else if let Some(folder_icon) = ctx.folder_icon_texture {
                ui.painter().image(
                    folder_icon.id(),
                    icon_rect,
                    Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            } else {
                ui.painter().text(
                    icon_rect.min,
                    egui::Align2::LEFT_TOP,
                    "\u{ED9F}", // ICON_FOLDER
                    FontId::new(14.0, egui::FontFamily::Name("icons".into())),
                    Color32::from_rgb(255, 193, 7),
                );
            }
        } else {
            // File: load native Windows icon using IconLoader (same as grid view)
            if let Some(file_icon) =
                ctx.item_icon_loader
                    .get_or_load_icon(ui.ctx(), &item.path, item.is_dir, false)
            {
                ui.painter().image(
                    file_icon.id(),
                    icon_rect,
                    Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            } else {
                // If icon not in cache and not loading, request it (async)
                if !ctx.loading_icons.contains(&item.path) && !ctx.failed_icons.contains(&item.path)
                {
                    ops.request_icon_load(item.path.clone());
                }

                ui.painter().text(
                    icon_rect.min,
                    egui::Align2::LEFT_TOP,
                    "\u{ECD3}", // ICON_FILE
                    FontId::new(14.0, egui::FontFamily::Name("icons".into())),
                    Color32::GRAY,
                );
            }
        }

        // RENAMING LOGIC (LIST VIEW)
        let is_renaming_this = ctx
            .renaming_state
            .as_ref()
            .is_some_and(|(idx, _)| *idx == i);
        if is_renaming_this {
            let mut text = ctx.renaming_state.as_ref().unwrap().1.clone();
            let name_rect = Rect::from_min_size(
                rect.min + egui::vec2(24.0, 2.0),
                egui::vec2(w_name - 30.0, row_height - 4.0),
            );

            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(name_rect), |ui| {
                let response = ui.add(
                    egui::TextEdit::singleline(&mut text)
                        .frame(true)
                        .horizontal_align(egui::Align::Min)
                        .id_source("rename_input_list"),
                );

                if ctx.focus_rename {
                    response.request_focus();
                }

                // Confirma renomeação com Enter (enquanto tem foco)
                if response.has_focus() && ui.input(|i_in| i_in.key_pressed(egui::Key::Enter)) {
                    ops.rename_with_shell(i);
                }
            });
        } else {
            // Name (truncated to fit column precisely)
            let font_id = FontId::proportional(12.0);
            let available_name_width = w_name - 30.0; // Space for icon + padding
            let display_name = truncate_text_for_column(&item.name, available_name_width, &font_id, ui);
            
            ui.painter().text(
                rect.min + egui::vec2(24.0, 5.0),
                egui::Align2::LEFT_TOP,
                display_name,
                font_id,
                text_color,
            );
        }

        if ctx.is_computer_view {
            // Computer View: Name, Espaço Total (w_date), Espaço Livre (w_size)
            // NO Type column should be displayed
            
            // 2. Total Size (Espaço Total) - positioned at w_name
            let total_str = if let Some(di) = &item.drive_info {
                format_size(di.total_space)
            } else {
                "-".to_string()
            };
            ui.painter().text(
                Pos2::new(rect.min.x + w_name, rect.min.y + 5.0),
                egui::Align2::LEFT_TOP,
                total_str,
                FontId::proportional(12.0),
                secondary_color,
            );

            // 3. Free Space (Espaço Livre) - positioned at w_name + w_date
            let free_str = if let Some(di) = &item.drive_info {
                format_size(di.free_space)
            } else {
                "-".to_string()
            };
            ui.painter().text(
                Pos2::new(rect.min.x + w_name + w_date, rect.min.y + 5.0),
                egui::Align2::LEFT_TOP,
                free_str,
                FontId::proportional(12.0),
                secondary_color,
            );
        } else {
            // 2. Date (truncated)
            let date_str = if ctx.is_recycle_bin_view {
                item.deletion_date
                    .clone()
                    .unwrap_or_else(|| "-".to_string())
            } else {
                format_date(item.modified)
            };
            let font_id = FontId::proportional(12.0);
            let available_date_width = w_date - 8.0; // Padding
            let display_date = truncate_text_for_column(&date_str, available_date_width, &font_id, ui);
            
            ui.painter().text(
                Pos2::new(rect.min.x + w_name, rect.min.y + 5.0),
                egui::Align2::LEFT_TOP,
                display_date,
                font_id.clone(),
                secondary_color,
            );

            // 3. Type (truncated precisely)
            let type_str = get_file_type_string(item);
            let available_type_width = w_type - 8.0; // Padding
            let display_type = truncate_text_for_column(&type_str, available_type_width, &font_id, ui);
            
            ui.painter().text(
                Pos2::new(rect.min.x + w_name + w_date, rect.min.y + 5.0),
                egui::Align2::LEFT_TOP,
                display_type,
                font_id.clone(),
                secondary_color,
            );

            // 4. Size
            let is_zip = item.name.to_lowercase().ends_with(".zip");
            let size_str = if item.is_dir && !is_zip {
                "".to_string()
            } else {
                format_size(item.size)
            };
            ui.painter().text(
                Pos2::new(rect.min.x + w_name + w_date + w_type, rect.min.y + 5.0),
                egui::Align2::LEFT_TOP,
                size_str,
                FontId::proportional(12.0),
                secondary_color,
            );

            // 5. OneDrive Status (if in OneDrive folder)
            if ctx.is_onedrive_folder {
                render_status_badge(
                    ui,
                    Pos2::new(
                        rect.min.x + w_name + w_date + w_type + w_size + 8.0,
                        rect.min.y + 4.0,
                    ),
                    item.sync_status,
                );
            }
        }
    });
}

// Header helper
fn render_section_header(ui: &mut Ui, title: &str) {
    ui.add_space(8.0);
    ui.label(
        RichText::new(title)
            .size(11.0)
            .color(Color32::from_gray(120))
            .strong(),
    );
    ui.add_space(4.0);
}

/// Helper function to get file type string
fn get_file_type_string(item: &FileEntry) -> String {
    // Check for Zip manually because is_dir might be true
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

/// Renders a sync status badge (OneDrive) in the status column
fn render_status_badge(ui: &mut egui::Ui, pos: Pos2, status: SyncStatus) {
    if status == SyncStatus::None {
        return; // No badge for normal files
    }

    let badge_size = 16.0;
    let badge_center = pos + egui::vec2(badge_size / 2.0, badge_size / 2.0);
    let badge_radius = badge_size / 2.0;

    let painter = ui.painter();

    match status {
        SyncStatus::CloudOnly => {
            // Blue cloud icon - file needs download
            painter.circle_filled(badge_center, badge_radius, Color32::from_rgb(0, 120, 215));
            painter.text(
                badge_center,
                egui::Align2::CENTER_CENTER,
                "☁",
                FontId::proportional(11.0),
                Color32::WHITE,
            );
        }
        SyncStatus::Syncing => {
            // Blue circular arrows - file is being synced
            painter.circle_filled(badge_center, badge_radius, Color32::from_rgb(0, 120, 215));
            painter.text(
                badge_center,
                egui::Align2::CENTER_CENTER,
                "⟳",
                FontId::proportional(12.0),
                Color32::WHITE,
            );
        }
        SyncStatus::Pinned => {
            // Green solid circle with check - always keep on device
            painter.circle_filled(badge_center, badge_radius, Color32::from_rgb(0, 150, 0));
            painter.text(
                badge_center,
                egui::Align2::CENTER_CENTER,
                "✓",
                FontId::proportional(10.0),
                Color32::WHITE,
            );
        }
        SyncStatus::LocallyAvailable => {
            // White circle with green outline/check - downloaded on demand
            painter.circle_filled(badge_center, badge_radius, Color32::WHITE);
            painter.circle_stroke(
                badge_center,
                badge_radius - 1.0,
                egui::Stroke::new(2.0, Color32::from_rgb(0, 150, 0)),
            );
            painter.text(
                badge_center,
                egui::Align2::CENTER_CENTER,
                "✓",
                FontId::proportional(10.0),
                Color32::from_rgb(0, 150, 0),
            );
        }
        SyncStatus::None => {} // Already handled above
    }
}
