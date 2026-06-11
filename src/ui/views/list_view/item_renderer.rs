//! Individual list item rendering: icons, columns, selection, tooltips, rename

use eframe::egui::{self, Color32, FontId, Pos2, Rect, Sense, Ui};

use super::helpers::{get_file_type_string, render_status_badge};
use super::item_renderer_details::{render_item_icon, render_item_tooltip};
use super::{truncate_text_for_column, ColumnWidths, ListViewContext, ListViewOperations};
use crate::domain::file_entry::FileEntry;
use crate::infrastructure::windows::{format_date, format_size};

/// Renders a single list item row
#[allow(clippy::too_many_arguments)]
pub(super) fn render_list_item(
    ui: &mut Ui,
    i: usize,
    item: &FileEntry,
    rect: Rect,
    ctx: &mut ListViewContext,
    ops: &mut dyn ListViewOperations,
    clicked_item: &mut Option<usize>,
    double_clicked_item: &mut Option<usize>,
    secondary_clicked_item: &mut Option<usize>,
    col_widths: &ColumnWidths,
    row_height: f32,
) {
    // LAZY LOAD TRIGGER FOR FOLDERS: Discover cover if not yet available
    if item.is_dir
        && !ctx.is_computer_view
        && !ctx.is_recycle_bin_view
        && item.folder_cover.is_none()
        && ctx.scanned_folders.peek(&item.path).is_none()
    {
        ctx.scanned_folders.put(item.path.clone(), ());
        ops.request_folder_scan(item.path.clone());
    }

    // LAZY LOAD TRIGGER FOR MEDIA FILES: Proactively load thumbnail
    if !item.is_dir && !ctx.is_recycle_bin_view {
        // PERFORMANCE: Use is_media() method to avoid registry lookups
        let is_selected_for_preview = ctx
            .selected_file
            .is_some_and(|selected| selected.path == item.path);

        // List view does not display thumbnails in rows, but we apply the
        // same bucket-aware thumbnail loading as grid view.  This ensures the
        // RAM cache always holds data at bucket 512+ for every visible media
        // file — so when the user clicks any file the detail panel's 512 px
        // request can be served from RAM cache immediately (fast path) rather
        // than going to the worker from scratch.

        if item.is_media()
            && !ctx.failed_thumbnails.contains(&item.path)
            && ctx.loading_set.len() < crate::ui::cache::MAX_THUMBNAIL_LOADING_SET_ITEMS
        {
            let is_loading = ctx.loading_set.contains(&item.path);
            let is_pending = ctx.pending_upload_set.contains(&item.path);
            let has_texture = ctx.texture_cache.peek(&item.path).is_some();

            // Mirror the bucket-aware sizing logic from grid view's
            // file_slot.rs so the RAM cache is populated at the same
            // resolution.  Grid view uses MIN_GRID_THUMBNAIL_BUCKET (512)
            // as the floor; list view does the same.
            let ppp = ui.ctx().pixels_per_point().max(1.0);
            let display_request_size_px: u32 = if ctx.show_preview_panel && is_selected_for_preview
            {
                crate::domain::thumbnail::detail_preview_size(&item.path)
            } else {
                // List rows don't show thumbnails — the display size is
                // irrelevant.  Use a minimal size; the bucket floor
                // below will upgrade it to 512.
                1
            };
            let display_effective_size_px = ((display_request_size_px as f32) * ppp).ceil() as u32;
            let display_bucket =
                crate::workers::thumbnail::processing::get_bucket_size(display_effective_size_px);
            let desired_thumbnail_bucket =
                display_bucket.max(crate::ui::theme::MIN_GRID_THUMBNAIL_BUCKET);
            let min_effective_size_for_bucket = match desired_thumbnail_bucket {
                0..=128 => 1,
                129..=256 => 129,
                257..=512 => 257,
                _ => 513,
            };
            let min_request_size_for_bucket =
                ((min_effective_size_for_bucket as f32) / ppp).ceil() as u32;
            let request_size_px = display_request_size_px.max(min_request_size_for_bucket);

            let attempted_bucket = ctx.attempted_thumbnail_bucket.get(&item.path).copied();
            let needs_bucket_refresh = match attempted_bucket {
                Some(b) => b < desired_thumbnail_bucket,
                None => false,
            };

            const MAX_THUMBNAIL_REQUESTS_PER_FRAME: usize = 96;

            let can_request = (!has_texture || needs_bucket_refresh)
                && !is_loading
                && !is_pending
                && ctx.thumbnail_requests_this_frame < MAX_THUMBNAIL_REQUESTS_PER_FRAME;

            if can_request {
                ctx.thumbnail_requests_this_frame += 1;
                ctx.loading_set.insert(item.path.clone());
                ops.request_thumbnail_load_with_size(
                    item.path.clone(),
                    request_size_px,
                    i,
                    item.modified,
                );
            }
        }
    }

    let is_recycle_bin = ctx.is_recycle_bin_view;
    let w_name = col_widths.name;
    let w_date = col_widths.date;
    let w_type = col_widths.type_col;
    let w_size = col_widths.size;

    ui.push_id(i, |ui| {
        let hidden_opacity = if item.is_hidden { 0.5 } else { 1.0 };
        let response = ui.interact(rect, ui.id().with(i), Sense::click_and_drag());

        if response.clicked() {
            *clicked_item = Some(i);
        }

        if response.double_clicked() {
            *double_clicked_item = Some(i);
        }

        if response.secondary_clicked() {
            *secondary_clicked_item = Some(i);
        }
        let (press_origin, pointer_pos) =
            ui.input(|i| (i.pointer.press_origin(), i.pointer.hover_pos()));
        let drag_candidate = crate::ui::views::common::should_start_item_drag(
            response.drag_started(),
            response.dragged(),
            response.is_pointer_button_down_on(),
            press_origin,
            pointer_pos,
        );
        let rectangle_select_active = ctx.rectangle_selection_state.is_some();
        if drag_candidate && !rectangle_select_active {
            if ctx.is_computer_view {
                *ctx.drag_started_item = Some(i);
            } else if let Some(origin) = ui.input(|input| input.pointer.press_origin()) {
                if list_item_content_contains_pointer(
                    ui, item, ctx, rect, col_widths, row_height, origin,
                ) {
                    *ctx.drag_started_item = Some(i);
                } else {
                    ctx.rectangle_selection_frame.request_start(origin);
                }
            } else {
                *ctx.drag_started_item = Some(i);
            }
        }
        let is_pointer_over = response.contains_pointer() || response.hovered();
        // For drag-hover detection use ONLY contains_pointer() (geometric check).
        // response.hovered() stays locked to the drag-source widget in egui,
        // so when the source is rendered AFTER the real target (target is above),
        // it would overwrite drag_hovered_item → wrong target → denied cursor.
        if response.contains_pointer() && item.is_dir {
            *ctx.drag_hovered_item = Some(i);
        }

        // --- VISUAL FEEDBACK: BORDER-ONLY (MODERN DESIGN) ---
        let is_selected = ctx
            .rectangle_selection_state
            .map(|state| state.preview_contains(i))
            .unwrap_or_else(|| ctx.multi_selection.contains(&item.path));

        // STRICT HOVER LOGIC: Only allow hover if LastInput was Mouse
        let allow_hover = matches!(ctx.last_input, crate::app::state::LastInput::Mouse);
        let is_hovered_visual = allow_hover && response.hovered() && !is_selected;

        let is_focused = ctx.selected_item == Some(i);

        let rounding = 4.0;
        let accent_color = crate::ui::theme::COLOR_ACCENT;

        // ADJUST RECT TO AVOID SCROLLBAR OVERLAP
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
            let hover_color = accent_color.gamma_multiply(0.35);
            ui.painter().rect_stroke(
                visual_rect,
                rounding,
                egui::Stroke::new(1.0, hover_color),
                egui::StrokeKind::Inside,
            );
        }

        let pointer_over_drop_candidate = ctx.is_item_dragging && item.is_dir && is_pointer_over;
        let is_active_drop_target = ctx.is_item_dragging
            && item.is_dir
            && ctx
                .drag_target_folder
                .as_ref()
                .is_some_and(|target| *target == item.path);

        if pointer_over_drop_candidate || is_active_drop_target {
            let stroke_color = if is_active_drop_target {
                Color32::from_rgb(24, 122, 255)
            } else {
                accent_color.gamma_multiply(0.75)
            };
            ui.painter().rect_stroke(
                visual_rect.shrink(1.0),
                rounding,
                egui::Stroke::new(2.0, stroke_color),
                egui::StrokeKind::Inside,
            );
        }

        // PERFORMANCE: Tooltip with debounce to avoid spam during scroll
        // Suppress tooltips during item drag to avoid clutter with drag ghost
        if !ctx.is_item_dragging && !rectangle_select_active {
            render_item_tooltip(ui, &response, item, ctx, is_recycle_bin);
        }

        let text_color = if is_selected {
            crate::ui::theme::selection_text_color(ui.visuals().dark_mode)
        } else {
            crate::ui::theme::text_color(ui.visuals().dark_mode)
        }
        .gamma_multiply(hidden_opacity);
        let secondary_color = if is_selected {
            crate::ui::theme::selection_text_color(ui.visuals().dark_mode)
        } else {
            crate::ui::theme::secondary_text_color(ui.visuals().dark_mode)
        }
        .gamma_multiply(hidden_opacity);

        // 1. Icon + Name
        let icon_tint = Color32::WHITE.gamma_multiply(hidden_opacity);
        render_item_icon(ui, item, ctx, ops, rect, icon_tint);

        // RENAMING LOGIC (LIST VIEW)
        let is_renaming_this = ctx
            .renaming_state
            .as_ref()
            .is_some_and(|(idx, _)| *idx == i);
        if is_renaming_this {
            let Some((_, ref rename_text)) = ctx.renaming_state else {
                return;
            };
            let mut text = rename_text.clone();
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

                    // Select name without extension (Windows Explorer behavior)
                    if let Some(mut state) =
                        egui::widgets::text_edit::TextEditState::load(ui.ctx(), response.id)
                    {
                        let char_count = text.chars().count();
                        let select_end = if item.is_dir {
                            char_count
                        } else {
                            text.rfind('.')
                                .map(|byte_pos| text[..byte_pos].chars().count())
                                .filter(|&pos| pos > 0)
                                .unwrap_or(char_count)
                        };
                        state
                            .cursor
                            .set_char_range(Some(egui::text::CCursorRange::two(
                                egui::text::CCursor::new(0),
                                egui::text::CCursor::new(select_end),
                            )));
                        state.store(ui.ctx(), response.id);
                    }
                }

                // Confirm rename with Enter (while focused)
                if ui.input(|i_in| i_in.key_pressed(egui::Key::Enter)) {
                    ops.rename_with_shell(i);
                }
            });

            // Persist edited text back to rename state (same behavior as grid mode).
            if let Some((_, rename_text)) = ctx.renaming_state.as_mut() {
                *rename_text = text;
            }
        } else {
            // Name (truncated to fit column precisely)
            let font_id = FontId::proportional(12.0);
            let available_name_width = w_name - 30.0; // Space for icon + padding
            let resolved_name = crate::ui::components::item_slot::display_name_for_item(item);
            let display_name =
                truncate_text_for_column(&resolved_name, available_name_width, &font_id, ui);

            ui.painter().text(
                rect.min + egui::vec2(24.0, 5.0),
                egui::Align2::LEFT_TOP,
                display_name,
                font_id,
                text_color,
            );
        }

        // Column data
        if ctx.is_computer_view {
            render_computer_view_columns(ui, item, rect, w_name, w_date, secondary_color);
        } else {
            render_regular_columns(
                ui,
                item,
                ctx,
                rect,
                w_name,
                w_date,
                w_type,
                w_size,
                secondary_color,
                is_recycle_bin,
            );
        }
    });
}

fn list_item_content_contains_pointer(
    ui: &Ui,
    item: &FileEntry,
    ctx: &ListViewContext,
    rect: Rect,
    col_widths: &ColumnWidths,
    row_height: f32,
    point: Pos2,
) -> bool {
    let icon_rect = Rect::from_min_size(rect.min + egui::vec2(4.0, 4.0), egui::vec2(16.0, 16.0));
    if icon_rect.expand(2.0).contains(point) {
        return true;
    }

    let font_id = FontId::proportional(12.0);
    let resolved_name = crate::ui::components::item_slot::display_name_for_item(item);
    if text_content_contains(
        ui,
        resolved_name.as_ref(),
        col_widths.name - 30.0,
        font_id.clone(),
        rect.min + egui::vec2(24.0, 5.0),
        row_height,
        point,
    ) {
        return true;
    }

    if ctx.is_computer_view {
        let total_str = item
            .drive_info
            .as_ref()
            .map(|drive| format_size(drive.total_space))
            .unwrap_or_else(|| "-".to_string());
        if text_content_contains(
            ui,
            &total_str,
            col_widths.date - 8.0,
            font_id.clone(),
            Pos2::new(rect.min.x + col_widths.name, rect.min.y + 5.0),
            row_height,
            point,
        ) {
            return true;
        }

        let free_str = item
            .drive_info
            .as_ref()
            .map(|drive| format_size(drive.free_space))
            .unwrap_or_else(|| "-".to_string());
        return text_content_contains(
            ui,
            &free_str,
            col_widths.size - 8.0,
            font_id,
            Pos2::new(
                rect.min.x + col_widths.name + col_widths.date,
                rect.min.y + 5.0,
            ),
            row_height,
            point,
        );
    }

    let date_str = if ctx.is_recycle_bin_view {
        if item.modified > 0 {
            format_date(item.modified)
        } else {
            item.deletion_date()
                .map(|s| s.to_string())
                .unwrap_or_else(|| "-".to_string())
        }
    } else {
        format_date(item.modified)
    };
    if text_content_contains(
        ui,
        &date_str,
        col_widths.date - 8.0,
        font_id.clone(),
        Pos2::new(rect.min.x + col_widths.name, rect.min.y + 5.0),
        row_height,
        point,
    ) {
        return true;
    }

    let type_str = get_file_type_string(item);
    if text_content_contains(
        ui,
        &type_str,
        col_widths.type_col - 8.0,
        font_id.clone(),
        Pos2::new(
            rect.min.x + col_widths.name + col_widths.date,
            rect.min.y + 5.0,
        ),
        row_height,
        point,
    ) {
        return true;
    }

    let size_str = if item.is_dir && !item.is_archive() {
        ctx.folder_size_cache
            .peek(&item.path)
            .map(|size| format_size(*size))
            .unwrap_or_default()
    } else {
        format_size(item.size)
    };
    if text_content_contains(
        ui,
        &size_str,
        col_widths.size - 8.0,
        font_id,
        Pos2::new(
            rect.min.x + col_widths.name + col_widths.date + col_widths.type_col,
            rect.min.y + 5.0,
        ),
        row_height,
        point,
    ) {
        return true;
    }

    if ctx.is_onedrive_folder {
        let status_rect = Rect::from_min_size(
            Pos2::new(
                rect.min.x
                    + col_widths.name
                    + col_widths.date
                    + col_widths.type_col
                    + col_widths.size
                    + 8.0,
                rect.min.y + 4.0,
            ),
            egui::vec2(18.0, 16.0),
        );
        return status_rect.expand(2.0).contains(point);
    }

    false
}

fn text_content_contains(
    ui: &Ui,
    text: &str,
    max_width: f32,
    font_id: FontId,
    origin: Pos2,
    row_height: f32,
    point: Pos2,
) -> bool {
    let max_width = max_width.max(0.0);
    if text.is_empty() || max_width <= 0.0 {
        return false;
    }

    let display_text = truncate_text_for_column(text, max_width, &font_id, ui);
    if display_text.is_empty() {
        return false;
    }

    let width = ui.fonts(|fonts| {
        fonts
            .layout_no_wrap(display_text, font_id, Color32::WHITE)
            .rect
            .width()
    });
    let hit_rect = Rect::from_min_size(
        origin,
        egui::vec2(width.min(max_width), (row_height - 6.0).max(12.0)),
    );
    hit_rect.expand(3.0).contains(point)
}

/// Renders columns for Computer View (Total Space, Free Space)
fn render_computer_view_columns(
    ui: &mut Ui,
    item: &FileEntry,
    rect: Rect,
    w_name: f32,
    w_date: f32,
    secondary_color: Color32,
) {
    // 2. Total Size - positioned at w_name
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

    // 3. Free Space - positioned at w_name + w_date
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
}

/// Renders columns for regular view (Date, Type, Size, OneDrive status)
#[allow(clippy::too_many_arguments)]
fn render_regular_columns(
    ui: &mut Ui,
    item: &FileEntry,
    ctx: &mut ListViewContext,
    rect: Rect,
    w_name: f32,
    w_date: f32,
    w_type: f32,
    w_size: f32,
    secondary_color: Color32,
    is_recycle_bin: bool,
) {
    // 2. Date (truncated)
    let date_str = if is_recycle_bin {
        if item.modified > 0 {
            format_date(item.modified)
        } else {
            item.deletion_date()
                .map(|s| s.to_string())
                .unwrap_or_else(|| "-".to_string())
        }
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
    let size_str = if item.is_dir && !item.is_archive() {
        // Folder: look up cached size from the batch worker.
        if let Some(&size) = ctx.folder_size_cache.peek(&item.path) {
            format_size(size)
        } else {
            // Request size if not already loading (collected by caller after render).
            if !ctx.folder_size_batch_loading.contains(&item.path) {
                ctx.folder_size_requests.push(item.path.clone());
            }
            String::new()
        }
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
