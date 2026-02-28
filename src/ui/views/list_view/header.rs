//! List view column header rendering with sortable, resizable columns

use eframe::egui::{self, Color32, FontId, Sense, Ui};

use super::{truncate_text_for_column, ListViewContext};
use crate::domain::file_entry::SortMode;

/// Draws a single resizable header column.
/// Returns true if the column was clicked (for sorting).
#[allow(clippy::too_many_arguments)]
fn draw_header_resizable(
    ui: &mut Ui,
    text: &str,
    width: &mut f32,
    mode: SortMode,
    min_width: f32,
    other_widths: f32,
    sort_mode: SortMode,
    sort_descending: bool,
    available_for_columns: f32,
) -> bool {
    let header_rect = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(*width, 22.0));

    // Header clickable area (for sorting)
    let header_id = ui.id().with(("header", text));
    let header_response = ui.interact(header_rect, header_id, Sense::click());

    let is_active = sort_mode == mode;

    if ui.is_rect_visible(header_rect) {
        if is_active {
            ui.painter()
                .rect_filled(header_rect, 2.0, Color32::from_gray(230));
        }
        let text_color = if is_active {
            Color32::BLACK
        } else {
            Color32::from_gray(100)
        };

        // Truncate text to fit within column
        let available_text_width = *width - 30.0; // Reserve space for arrow and padding
        let font_id = FontId::proportional(12.0);
        // PERFORMANCE: Reuse binary-search truncation instead of linear char-by-char loop
        let display_text = truncate_text_for_column(text, available_text_width, &font_id, ui);

        ui.painter().text(
            header_rect.min + egui::vec2(8.0, 4.0),
            egui::Align2::LEFT_TOP,
            display_text,
            font_id,
            text_color,
        );

        if is_active {
            let arrow = if sort_descending { "▼" } else { "▲" };
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
        egui::vec2(handle_width, 22.0),
    );

    let handle_id = ui.id().with(("resize", text));
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
            Color32::from_rgb(100, 150, 200),
        );
    }

    // Advance cursor
    ui.allocate_exact_size(egui::vec2(*width, 22.0), Sense::hover());

    header_response.clicked()
}

/// Renders the full list header row with all columns.
/// Returns Some(SortMode) if a column header was clicked for sorting.
pub(super) fn render_list_header(
    ui: &mut Ui,
    ctx: &mut ListViewContext,
    available_w: f32,
) -> Option<SortMode> {
    let sort_mode = ctx.sort_mode;
    let sort_descending = ctx.sort_descending;
    let w_status = if ctx.is_onedrive_folder && !ctx.is_computer_view {
        *ctx.col_status_width
    } else {
        0.0
    };

    let mut sort_action: Option<SortMode> = None;

    let maybe_sort_action = ui
        .horizontal(|ui| {
            ui.style_mut().spacing.item_spacing.x = 0.0;

            // Calculate available space for columns (total - scrollbar - status column)
            let available_for_columns = available_w - 8.0 - w_status;

            // Calculate current widths for constraint checks
            let current_date = *ctx.col_date_width;
            let current_size = *ctx.col_size_width;

            if draw_header_resizable(
                ui,
                "Nome",
                ctx.col_name_width,
                SortMode::Name,
                100.0,
                current_date + current_size,
                sort_mode,
                sort_descending,
                available_for_columns,
            ) {
                return Some(SortMode::Name);
            }

            if ctx.is_computer_view {
                // Computer View: only Name, Total Space and Free Space (no Type)
                // Recalculate after potential Name resize
                let current_name = *ctx.col_name_width;
                let current_size = *ctx.col_size_width;

                if draw_header_resizable(
                    ui,
                    "Espaço Total",
                    ctx.col_date_width,
                    SortMode::DriveTotalSpace,
                    80.0,
                    current_name + current_size,
                    sort_mode,
                    sort_descending,
                    available_for_columns,
                ) {
                    return Some(SortMode::DriveTotalSpace);
                }

                // Recalculate after potential Date resize
                let current_name = *ctx.col_name_width;
                let current_date = *ctx.col_date_width;

                if draw_header_resizable(
                    ui,
                    "Espaço Livre",
                    ctx.col_size_width,
                    SortMode::DriveFreeSpace,
                    80.0,
                    current_name + current_date,
                    sort_mode,
                    sort_descending,
                    available_for_columns,
                ) {
                    return Some(SortMode::DriveFreeSpace);
                }
            } else {
                // Regular view: Name, Date, Type, Size (+ Status if OneDrive)
                // Recalculate after potential Name resize
                let current_name = *ctx.col_name_width;
                let current_type = *ctx.col_type_width;
                let current_size = *ctx.col_size_width;

                let date_label = if ctx.is_recycle_bin_view {
                    "Data de Exclusão"
                } else {
                    "Última modificação"
                };
                if draw_header_resizable(
                    ui,
                    date_label,
                    ctx.col_date_width,
                    SortMode::Date,
                    120.0,
                    current_name + current_type + current_size,
                    sort_mode,
                    sort_descending,
                    available_for_columns,
                ) {
                    return Some(SortMode::Date);
                }

                // Recalculate after potential Date resize
                let current_name = *ctx.col_name_width;
                let current_date = *ctx.col_date_width;
                let current_size = *ctx.col_size_width;

                if draw_header_resizable(
                    ui,
                    "Tipo",
                    ctx.col_type_width,
                    SortMode::Type,
                    80.0,
                    current_name + current_date + current_size,
                    sort_mode,
                    sort_descending,
                    available_for_columns,
                ) {
                    return Some(SortMode::Type);
                }

                // Recalculate after potential Type resize
                let current_name = *ctx.col_name_width;
                let current_date = *ctx.col_date_width;
                let current_type = *ctx.col_type_width;

                if draw_header_resizable(
                    ui,
                    "Tamanho",
                    ctx.col_size_width,
                    SortMode::Size,
                    80.0,
                    current_name + current_date + current_type,
                    sort_mode,
                    sort_descending,
                    available_for_columns,
                ) {
                    return Some(SortMode::Size);
                }

                // Status column (OneDrive only) - now resizable
                if ctx.is_onedrive_folder {
                    render_status_header(ui, ctx, available_w);
                }
            }

            None
        })
        .inner;

    if let Some(mode) = maybe_sort_action {
        sort_action = Some(mode);
    }

    sort_action
}

/// Renders the OneDrive status column header (no sorting, but resizable)
fn render_status_header(ui: &mut Ui, ctx: &mut ListViewContext, available_w: f32) {
    let current_name = *ctx.col_name_width;
    let current_date = *ctx.col_date_width;
    let current_type = *ctx.col_type_width;
    let current_size = *ctx.col_size_width;

    let header_rect =
        egui::Rect::from_min_size(ui.cursor().min, egui::vec2(*ctx.col_status_width, 22.0));

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
        egui::vec2(handle_width, 22.0),
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
            Color32::from_rgb(100, 150, 200),
        );
    }

    ui.allocate_exact_size(egui::vec2(*ctx.col_status_width, 22.0), Sense::hover());
}
