//! "Efficiency" tab: logical-vs-allocated divergence (plan section 6).
//!
//! Two explicit groups — files where the logical size exceeds allocation
//! (sparse/compressed/resident-like) and files where allocation exceeds the
//! logical size (cluster slack / ADS-like). The model does not preserve
//! attributes, so no file is ever *labelled* sparse or compressed.

use super::{
    analyzer_text_color, attach_row_menu,
    largest_items::{row_background, weak_text},
    paint_cell, panel_secondary_text_color, HEADER_ROW_H, ROW_HEIGHT,
};
use crate::app::disk_analysis_model::DiskAnalysisModel;
use crate::app::disk_analysis_query::EfficiencyRow;
use crate::app::disk_analysis_state::DiskAnalysisState;
use crate::infrastructure::windows::formatting::format_size;
use crate::ui::disk_analysis::category_label;
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
    let Some(result) = state.efficiency_result.clone() else {
        ui.centered_and_justified(|ui| {
            ui.label(
                egui::RichText::new(t!("disk_analysis.efficiency_scanning").to_string())
                    .color(panel_secondary_text_color(ui)),
            );
        });
        return;
    };
    if result.logical_greater.is_empty() && result.allocated_greater.is_empty() {
        ui.centered_and_justified(|ui| {
            ui.label(
                egui::RichText::new(t!("disk_analysis.efficiency_empty").to_string())
                    .color(panel_secondary_text_color(ui)),
            );
        });
        return;
    }
    if result.truncated {
        ui.label(
            egui::RichText::new(t!("disk_analysis.list_truncated").to_string())
                .small()
                .color(panel_secondary_text_color(ui)),
        );
    }

    let total_w = ui.available_width();
    let widths = column_widths(total_w);
    paint_header(ui, total_w, widths);
    ui.separator();

    egui::ScrollArea::vertical()
        .id_salt("disk_analysis_efficiency_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            section(
                state,
                ui,
                &model,
                &result.logical_greater,
                t!("disk_analysis.efficiency_logical_greater").to_string(),
                t!(
                    "disk_analysis.efficiency_savings_total",
                    bytes = format_size(result.logical_greater_total)
                )
                .to_string(),
                widths,
            );
            ui.add_space(12.0);
            section(
                state,
                ui,
                &model,
                &result.allocated_greater,
                t!("disk_analysis.efficiency_allocated_greater").to_string(),
                t!(
                    "disk_analysis.efficiency_overhead_total",
                    bytes = format_size(result.allocated_greater_total)
                )
                .to_string(),
                widths,
            );
        });
}

fn paint_header(ui: &mut egui::Ui, total_w: f32, widths: [f32; 7]) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(total_w, HEADER_ROW_H), egui::Sense::hover());
    let columns = [
        (t!("disk_analysis.col_name").to_string(), false),
        (t!("disk_analysis.col_path").to_string(), false),
        (t!("disk_analysis.logical_size_short").to_string(), true),
        (t!("disk_analysis.allocated_size").to_string(), true),
        (t!("disk_analysis.col_difference").to_string(), true),
        (t!("disk_analysis.col_ratio").to_string(), true),
        (t!("disk_analysis.col_category").to_string(), false),
    ];
    let mut x = rect.left();
    for (slot, (label, right)) in columns.into_iter().enumerate() {
        let cell = egui::Rect::from_min_size(
            egui::pos2(x, rect.top()),
            egui::vec2(widths[slot], HEADER_ROW_H),
        );
        let (position, alignment) = if right {
            (
                egui::pos2(cell.right() - 6.0, cell.center().y),
                egui::Align2::RIGHT_CENTER,
            )
        } else {
            (
                egui::pos2(cell.left() + 6.0, cell.center().y),
                egui::Align2::LEFT_CENTER,
            )
        };
        ui.painter().text(
            position,
            alignment,
            label,
            egui::FontId::proportional(12.0),
            analyzer_text_color(ui),
        );
        x += widths[slot];
    }
}

fn section(
    state: &mut DiskAnalysisState,
    ui: &mut egui::Ui,
    model: &Arc<DiskAnalysisModel>,
    rows: &[EfficiencyRow],
    title: String,
    total_line: String,
    widths: [f32; 7],
) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(title).strong());
        ui.label(
            egui::RichText::new(total_line)
                .small()
                .color(panel_secondary_text_color(ui)),
        );
    });
    ui.add_space(2.0);

    let total_w = widths.iter().sum();
    for (i, row) in rows.iter().enumerate() {
        let resp = ui.allocate_response(egui::vec2(total_w, ROW_HEIGHT), egui::Sense::click());
        let bg = row_background(ui, state.selected == Some(row.idx), resp.hovered(), i);
        if bg != egui::Color32::TRANSPARENT {
            ui.painter().rect_filled(resp.rect, 3.0, bg);
        }
        paint_row(
            ui.painter(),
            resp.rect,
            model,
            row,
            widths,
            analyzer_text_color(ui),
            weak_text(ui),
        );

        if resp.clicked() {
            // Select + highlight in the treemap; scroll the Largest tab to
            // the same item when it appears there.
            state.reveal_file(row.idx);
            if let Some(pos) = state.largest_rows.iter().position(|&r| r == row.idx) {
                state.largest_scroll_to_row = Some(pos);
            }
        }
        attach_row_menu(state, ui, &resp, row.idx, false);
    }
    ui.add_space(4.0);
}

/// Name | Path | Logical | Allocated | Abs diff | Ratio | Category
fn column_widths(total_w: f32) -> [f32; 7] {
    let numeric = 84.0;
    let ratio_w = 64.0;
    let category_w = 92.0;
    let flex = (total_w - numeric * 3.0 - ratio_w - category_w).max(220.0);
    let name = (flex * 0.45).clamp(110.0, 280.0);
    let path = (flex - name).max(80.0);
    [name, path, numeric, numeric, numeric, ratio_w, category_w]
}

fn paint_row(
    painter: &egui::Painter,
    rect: egui::Rect,
    model: &Arc<DiskAnalysisModel>,
    row: &EfficiencyRow,
    widths: [f32; 7],
    text_color: egui::Color32,
    weak: egui::Color32,
) {
    let node = &model.nodes[row.idx as usize];
    let mut x = rect.left();

    paint_cell(
        painter, rect, &mut x, widths[0], &node.name, false, text_color,
    );
    paint_cell(
        painter,
        rect,
        &mut x,
        widths[1],
        &model.path_of(row.idx),
        false,
        weak,
    );
    paint_cell(
        painter,
        rect,
        &mut x,
        widths[2],
        &format_size(node.size),
        true,
        text_color,
    );
    paint_cell(
        painter,
        rect,
        &mut x,
        widths[3],
        &format_size(node.allocated_size),
        true,
        text_color,
    );
    paint_cell(
        painter,
        rect,
        &mut x,
        widths[4],
        &format_size(row.absolute_difference),
        true,
        weak,
    );
    let ratio_text = if node.allocated_size == 0 {
        "\u{221E}".to_string()
    } else {
        format!(
            "{:.2}\u{00D7}",
            node.size as f64 / node.allocated_size as f64
        )
    };
    paint_cell(painter, rect, &mut x, widths[5], &ratio_text, true, weak);
    paint_cell(
        painter,
        rect,
        &mut x,
        widths[6],
        &category_label(node.category),
        false,
        weak,
    );
}
