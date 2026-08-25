//! "Largest items" tab: top-K heaviest descendants of the current subtree
//! (plan section 4). Rows are plain node indices; all displayed values come
//! from the model at paint time.

use super::{
    analyzer_text_color, attach_row_menu, paint_cell, panel_secondary_text_color, HEADER_ROW_H,
    ROW_HEIGHT,
};
use crate::app::disk_analysis_model::DiskAnalysisModel;
use crate::app::disk_analysis_query::SizeMetric;
use crate::app::disk_analysis_state::{DiskAnalysisState, LargestColumn};
use crate::infrastructure::windows::formatting::format_size;
use eframe::egui;
use rust_i18n::t;
use std::sync::Arc;

pub fn render(state: &mut DiskAnalysisState, ui: &mut egui::Ui) {
    let Some(model) = state.model.clone() else {
        ui.centered_and_justified(|ui| {
            ui.label(t!("disk_analysis.no_model").to_string());
        });
        return;
    };

    let rows = state.largest_rows.clone();
    if rows.is_empty() {
        ui.centered_and_justified(|ui| {
            ui.label(
                egui::RichText::new(t!("disk_analysis.largest_empty").to_string())
                    .color(panel_secondary_text_color(ui)),
            );
        });
        return;
    }

    let total_w = ui.available_width();
    let widths = column_widths(total_w);

    paint_header(ui, total_w, widths, state);

    // One-shot scroll request from search activation (plan section 5).
    let scroll_to = state.largest_scroll_to_row.take();
    let mut scroll_area = egui::ScrollArea::vertical()
        .id_salt("disk_analysis_largest_scroll")
        .auto_shrink([false, false]);
    if let Some(row) = scroll_to {
        // Same technique as the text viewer: show_rows positions rows with
        // item spacing, so the stride includes it.
        let row_stride = ROW_HEIGHT + ui.spacing().item_spacing.y;
        scroll_area = scroll_area.vertical_scroll_offset(row as f32 * row_stride);
    }

    scroll_area.show_rows(ui, ROW_HEIGHT, rows.len(), |ui, range| {
        for row in range {
            let idx = rows[row];
            let resp = ui.allocate_response(egui::vec2(total_w, ROW_HEIGHT), egui::Sense::click());
            let bg = row_background(ui, state.selected == Some(idx), resp.hovered(), row);
            if bg != egui::Color32::TRANSPARENT {
                ui.painter().rect_filled(resp.rect, 3.0, bg);
            }
            paint_largest_row(
                ui.painter(),
                resp.rect,
                &model,
                idx,
                state.active_weights.as_ref().and_then(|weights| {
                    weights
                        .weights
                        .get(idx as usize)
                        .copied()
                        .map(|value| (state.metric, value))
                }),
                widths,
                (analyzer_text_color(ui), weak_text(ui)),
            );

            if resp.clicked() {
                state.selected = Some(idx);
            }
            // Double click on a folder navigates into it.
            if resp.double_clicked() && model.nodes[idx as usize].is_dir {
                state.navigate_to(idx);
                state.selected = None;
            }
            attach_row_menu(state, ui, &resp, idx, false);
        }
    });
}

pub(crate) fn row_background(
    ui: &egui::Ui,
    selected: bool,
    hovered: bool,
    zebra_index: usize,
) -> egui::Color32 {
    if selected {
        ui.visuals().selection.bg_fill.gamma_multiply(0.55)
    } else if hovered {
        ui.visuals().widgets.hovered.bg_fill
    } else if zebra_index % 2 == 1 {
        ui.visuals().faint_bg_color
    } else {
        egui::Color32::TRANSPARENT
    }
}

/// Name | Path | Type | Logical | Allocated | Difference | Files
fn column_widths(total_w: f32) -> [f32; 7] {
    let type_w = 56.0;
    let numeric = 88.0;
    let files_w = 64.0;
    let flex = (total_w - type_w - numeric * 3.0 - files_w).max(220.0);
    let name = (flex * 0.42).clamp(120.0, 320.0);
    let path = (flex - name).max(80.0);
    [name, path, type_w, numeric, numeric, numeric, files_w]
}

fn paint_header(ui: &mut egui::Ui, total_w: f32, widths: [f32; 7], state: &mut DiskAnalysisState) {
    let (allocated_rect, _) =
        ui.allocate_exact_size(egui::vec2(total_w, HEADER_ROW_H), egui::Sense::hover());
    // egui reserves item spacing below an allocated row. Treat that reserved
    // space as part of the header bar so the labels are optically centered in
    // the full area above the table instead of only in its upper 24 pixels.
    let header_rect = egui::Rect::from_min_max(
        allocated_rect.min,
        egui::pos2(
            allocated_rect.max.x,
            allocated_rect.max.y + ui.spacing().item_spacing.y,
        ),
    );
    let (_, separator) = crate::ui::theme::viewer_bar_colors(ui.visuals().dark_mode);
    ui.painter().hline(
        header_rect.x_range(),
        header_rect.bottom(),
        egui::Stroke::new(1.0, separator),
    );
    let columns = [
        (
            Some(LargestColumn::Name),
            t!("disk_analysis.col_name").to_string(),
            false,
        ),
        (
            Some(LargestColumn::Path),
            t!("disk_analysis.col_path").to_string(),
            false,
        ),
        (None, t!("disk_analysis.col_type").to_string(), false),
        (
            Some(LargestColumn::Logical),
            t!("disk_analysis.logical_size_short").to_string(),
            true,
        ),
        (
            Some(LargestColumn::Allocated),
            t!("disk_analysis.allocated_size").to_string(),
            true,
        ),
        (
            Some(LargestColumn::Difference),
            t!("disk_analysis.col_difference").to_string(),
            true,
        ),
        (
            Some(LargestColumn::Files),
            t!("disk_analysis.col_files").to_string(),
            true,
        ),
    ];
    let mut x = header_rect.left();
    for (slot, (column, label, right)) in columns.into_iter().enumerate() {
        let hit = egui::Rect::from_min_size(
            egui::pos2(x, header_rect.top()),
            egui::vec2(widths[slot], HEADER_ROW_H),
        );
        let arrow = match column {
            Some(c) if state.largest_sort_column == c => {
                if state.largest_sort_asc {
                    " ^"
                } else {
                    " v"
                }
            }
            _ => "",
        };
        let color = if column.is_some() {
            analyzer_text_color(ui)
        } else {
            panel_secondary_text_color(ui)
        };
        let align2 = if right {
            egui::Align2::RIGHT_CENTER
        } else {
            egui::Align2::LEFT_CENTER
        };
        let pos = if right {
            egui::pos2(hit.right() - 6.0, hit.center().y)
        } else {
            egui::pos2(hit.left() + 6.0, hit.center().y)
        };
        ui.painter().text(
            pos,
            align2,
            format!("{label}{arrow}"),
            egui::FontId::proportional(12.0),
            color,
        );

        if let Some(column) = column {
            let resp = ui.allocate_rect(hit, egui::Sense::click());
            if resp.clicked() {
                if state.largest_sort_column == column {
                    state.largest_sort_asc = !state.largest_sort_asc;
                } else {
                    state.largest_sort_column = column;
                    state.largest_sort_asc = matches!(
                        column,
                        LargestColumn::Logical
                            | LargestColumn::Allocated
                            | LargestColumn::Difference
                            | LargestColumn::Files
                    );
                }
                state.sort_largest_rows();
            }
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
        }
        x += widths[slot];
    }
}

fn paint_largest_row(
    painter: &egui::Painter,
    rect: egui::Rect,
    model: &Arc<DiskAnalysisModel>,
    idx: u32,
    filtered: Option<(SizeMetric, u64)>,
    widths: [f32; 7],
    colors: (egui::Color32, egui::Color32),
) {
    let (text_color, weak) = colors;
    let node = &model.nodes[idx as usize];
    let filtered_value = filtered.map(|(_, value)| value);
    let metric = filtered.map(|(metric, _)| metric);
    let mut x = rect.left();

    paint_cell(
        painter, rect, &mut x, widths[0], &node.name, false, text_color,
    );
    // Full paths are built only for painted rows (plan section 4).
    paint_cell(
        painter,
        rect,
        &mut x,
        widths[1],
        &model.path_of(idx),
        false,
        weak,
    );
    let type_text = if node.is_dir {
        t!("disk_analysis.type_folder").to_string()
    } else {
        t!("disk_analysis.type_file").to_string()
    };
    paint_cell(painter, rect, &mut x, widths[2], &type_text, false, weak);
    paint_cell(
        painter,
        rect,
        &mut x,
        widths[3],
        &format_size(if metric == Some(SizeMetric::Logical) {
            filtered_value.unwrap_or(node.subtree_size)
        } else {
            node.subtree_size
        }),
        true,
        text_color,
    );
    paint_cell(
        painter,
        rect,
        &mut x,
        widths[4],
        &format_size(if metric == Some(SizeMetric::Allocated) {
            filtered_value.unwrap_or(node.subtree_allocated_size)
        } else {
            node.subtree_allocated_size
        }),
        true,
        text_color,
    );
    let diff_text = signed_diff_text(node.subtree_size, node.subtree_allocated_size);
    paint_cell(painter, rect, &mut x, widths[5], &diff_text, true, weak);
    let files_text = if node.is_dir {
        if metric == Some(SizeMetric::FileCount) {
            filtered_value.unwrap_or(node.subtree_files).to_string()
        } else {
            node.subtree_files.to_string()
        }
    } else {
        String::new()
    };
    paint_cell(painter, rect, &mut x, widths[6], &files_text, true, weak);
}

/// Signed logical-minus-allocated difference rendered without any raw u64
/// subtraction (the branch picks the safe order).
pub(crate) fn signed_diff_text(logical: u64, allocated: u64) -> String {
    if logical == allocated {
        "0".to_string()
    } else if logical > allocated {
        format!("+{}", format_size(logical - allocated))
    } else {
        format!("\u{2212}{}", format_size(allocated - logical))
    }
}

/// Weak text color flattened to opaque (painter galley tinting expects it).
pub(crate) fn weak_text(ui: &egui::Ui) -> egui::Color32 {
    let c = panel_secondary_text_color(ui);
    egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 255)
}
