//! Individual list item rendering: icons, columns, selection, tooltips, rename

use eframe::egui::{self, Color32, FontId, Pos2, Rect, RichText, Sense, Ui};

use super::helpers::{get_file_type_string, render_status_badge};
use super::{truncate_text_for_column, ColumnWidths, ListViewContext, ListViewOperations};
use crate::domain::file_entry::FileEntry;
use crate::infrastructure::windows::{format_date, format_size};

#[derive(Clone, Copy)]
struct TooltipLiveFileStat {
    checked_at: f64,
    size: u64,
}

fn probe_file_size(path: &std::path::Path) -> Option<u64> {
    let metadata = if crate::infrastructure::onedrive::is_onedrive_path(path) {
        crate::infrastructure::onedrive::onedrive_metadata(path).ok()
    } else {
        std::fs::metadata(path).ok()
    }?;

    if metadata.is_file() {
        Some(metadata.len())
    } else {
        None
    }
}

fn resolve_tooltip_live_size(ui: &Ui, item: &FileEntry) -> u64 {
    if item.is_dir {
        return item.size;
    }

    let now = ui.input(|i| i.time);
    let cache_id = egui::Id::new("tooltip_live_file_size").with(&item.path);
    let mut resolved = item.size;

    ui.ctx().data_mut(|d| {
        let mut state = d
            .get_temp::<TooltipLiveFileStat>(cache_id)
            .unwrap_or(TooltipLiveFileStat {
                checked_at: -10.0,
                size: item.size,
            });

        if (now - state.checked_at) >= 1.0 {
            if let Some(size) = probe_file_size(&item.path) {
                state.size = size;
            } else {
                state.size = item.size;
            }
            state.checked_at = now;
            d.insert_temp(cache_id, state);
        }

        resolved = state.size;
    });

    resolved
    }

// PERFORMANCE: Tooltip debounce to avoid creation/destruction during scroll
const TOOLTIP_DELAY_SECS: f32 = 0.3;

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
        if item.is_media()
            && !ctx.texture_cache.contains(&item.path)
            && !ctx.loading_set.contains(&item.path)
            && !ctx.failed_thumbnails.contains(&item.path)
            && !ctx.pending_upload_set.contains(&item.path)
            && ctx.loading_set.len() < 200
        {
            ctx.loading_set.insert(item.path.clone());
            ops.request_thumbnail_load(item.path.clone(), i, item.modified);
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
        let pointer_moved = ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO);
        if response.drag_started()
            || response.dragged()
            || (response.is_pointer_button_down_on() && pointer_moved)
        {
            *ctx.drag_started_item = Some(i);
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
        let is_selected = ctx.multi_selection.contains(&item.path);

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
        if !ctx.is_item_dragging {
            render_item_tooltip(ui, &response, item, ctx, is_recycle_bin);
        }

        let text_color = if is_selected {
            crate::ui::theme::COLOR_SELECTION_TEXT
        } else {
            Color32::BLACK
        }.gamma_multiply(hidden_opacity);
        let secondary_color = if is_selected {
            crate::ui::theme::COLOR_SELECTION_TEXT
        } else {
            Color32::from_gray(100)
        }.gamma_multiply(hidden_opacity);

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
            let display_name =
                truncate_text_for_column(&item.name, available_name_width, &font_id, ui);

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

/// Renders tooltip with debounce for a list item
fn render_item_tooltip(
    ui: &mut Ui,
    response: &egui::Response,
    item: &FileEntry,
    ctx: &ListViewContext,
    is_recycle_bin: bool,
) {
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
            let screen_right = ui.ctx().screen_rect().right();
            let tooltip_width = 320.0;

            let effective_right = if ctx.is_video_docked_visible {
                screen_right * 0.72
            } else {
                screen_right
            };

            let tooltip_x = if mouse_pos.x + tooltip_width > effective_right {
                (effective_right - tooltip_width - 5.0).max(10.0)
            } else {
                mouse_pos.x
            };
            let tooltip_pos = egui::pos2(tooltip_x, mouse_pos.y);

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
                        ui.horizontal(|ui| {
                            ui.label("Tipo:");
                            ui.label(get_file_type_string(item));
                        });
                        if !item.is_dir || item.is_archive() {
                            ui.horizontal(|ui| {
                                ui.label("Tamanho:");
                                ui.label(format_size(resolve_tooltip_live_size(ui, item)));
                            });
                        }
                        let date_lbl = if is_recycle_bin {
                            "Data de Exclusão"
                        } else {
                            "Última modificação"
                        };
                        let date_val = if is_recycle_bin {
                            if item.modified > 0 {
                                format_date(item.modified)
                            } else {
                                item.deletion_date
                                    .clone()
                                    .unwrap_or_else(|| "-".to_string())
                            }
                        } else {
                            format_date(item.modified)
                        };
                        ui.horizontal(|ui| {
                            ui.label(date_lbl);
                            ui.label(":");
                            ui.label(date_val);
                        });
                    });
                },
            );
        }
    } else {
        // Clear hover time when not hovering
        let hover_id = response.id.with("hover_start");
        ui.ctx().data_mut(|d| d.remove::<f64>(hover_id));
    }
}

/// Renders the item icon (drive, folder, or file)
fn render_item_icon(
    ui: &mut Ui,
    item: &FileEntry,
    ctx: &mut ListViewContext,
    ops: &mut dyn ListViewOperations,
    rect: Rect,
    tint: Color32,
) {
    let icon_size_px = 16.0;
    let icon_rect = Rect::from_min_size(
        rect.min + egui::vec2(4.0, 4.0),
        egui::vec2(icon_size_px, icon_size_px),
    );

    if item.drive_info.is_some() {
        if let Some(drive_icon) = ctx
            .item_icon_loader
            .get_or_load_drive_icon(ui.ctx(), &item.path.to_string_lossy())
        {
            ui.painter().image(
                drive_icon.id(),
                icon_rect,
                Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                tint,
            );
        }
        return;
    }

    if item.is_dir && !item.is_archive() {
        let path_lower = item.path.to_string_lossy().to_lowercase();
        let is_virtual_archive =
            crate::domain::file_entry::path_contains_archive_segment(&path_lower);

        if is_virtual_archive {
            if let Some(folder_icon) =
                ctx.item_icon_loader
                    .get_or_load_icon(ui.ctx(), &item.path, true, false)
            {
                ui.painter().image(
                    folder_icon.id(),
                    icon_rect,
                    Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                    tint,
                );
                return;
            }
        }

        if let Some(folder_icon) = ctx.folder_icon_texture {
            ui.painter().image(
                folder_icon.id(),
                icon_rect,
                Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                tint,
            );
        }
        return;
    }

    if item.is_archive() {
        if let Some(file_icon) =
            ctx.item_icon_loader
                .get_or_load_icon(ui.ctx(), &item.path, false, false)
        {
            ui.painter().image(
                file_icon.id(),
                icon_rect,
                Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                tint,
            );
        } else if !ctx.loading_icons.contains(&item.path)
            && ctx.failed_icons.peek(&item.path).is_none()
        {
            ops.request_icon_load(item.path.clone());
        }
        return;
    }

    if let Some(file_icon) = ctx
        .item_icon_loader
        .get_or_load_icon(ui.ctx(), &item.path, item.is_dir, false)
    {
        ui.painter().image(
            file_icon.id(),
            icon_rect,
            Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
            tint,
        );
    } else if !ctx.loading_icons.contains(&item.path) && ctx.failed_icons.peek(&item.path).is_none()
    {
        ops.request_icon_load(item.path.clone());
    }
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
    ctx: &ListViewContext,
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
            item.deletion_date
                .clone()
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
