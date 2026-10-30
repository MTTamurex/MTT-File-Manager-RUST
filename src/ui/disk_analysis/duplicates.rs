//! "Duplicates" tab: manual duplicate detection UI (plan section 7).
//!
//! The scan only starts on an explicit button press; progress and the final
//! report render from [`DuplicateSession`]. Explicit deletion is available only
//! in the comparison view, with validation and native Windows confirmations.

use super::{
    analyzer_text_color, attach_row_menu, largest_items::weak_text, paint_cell,
    panel_secondary_text_color, ROW_HEIGHT,
};
use crate::app::disk_analysis_duplicates::{DuplicateReadMode, DuplicateReport};
use crate::app::disk_analysis_model::DiskAnalysisModel;
use crate::app::disk_analysis_state::DiskAnalysisState;
use crate::infrastructure::windows::formatting::format_size;
use eframe::egui;
use rust_i18n::t;
use std::sync::Arc;

mod details;
mod progress;
mod review;
use progress::{phase_label, render_progress};

pub fn render(state: &mut DiskAnalysisState, ui: &mut egui::Ui) {
    let Some(model) = state.model.clone() else {
        ui.centered_and_justified(|ui| {
            ui.label(t!("disk_analysis.no_model").to_string());
        });
        return;
    };

    render_controls(state, ui, &model);

    if state.duplicates.is_running() {
        ui.add_space(6.0);
        render_progress(state, ui, &model);
        return;
    }

    match state.duplicates.report.clone() {
        Some(report) => {
            state.duplicates.review.bind_report(&report);
            ui.add_space(6.0);
            render_report(state, ui, &model, &report);
            review::render(state, ui, &model, &report);
        }
        None => {
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(t!("disk_analysis.dup_idle_hint").to_string())
                    .color(panel_secondary_text_color(ui)),
            );
        }
    }
}

fn render_controls(
    state: &mut DiskAnalysisState,
    ui: &mut egui::Ui,
    model: &Arc<DiskAnalysisModel>,
) {
    let running = state.duplicates.is_running();
    if running {
        ui.horizontal_wrapped(|ui| {
            if ui
                .button(t!("disk_analysis.dup_cancel").to_string())
                .clicked()
            {
                state.duplicates.cancel();
                ui.ctx().request_repaint();
            }
            phase_label(ui, state.duplicates.phase);
        });
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.label(t!("disk_analysis.dup_read_mode").to_string());
        ui.add_enabled_ui(!running, |ui| {
            egui::ComboBox::from_id_salt("duplicate_read_mode")
                .selected_text(
                    match state.duplicates.read_mode {
                        DuplicateReadMode::Conservative => {
                            t!("disk_analysis.dup_read_conservative")
                        }
                        DuplicateReadMode::Parallel => t!("disk_analysis.dup_read_parallel"),
                    }
                    .to_string(),
                )
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut state.duplicates.read_mode,
                        DuplicateReadMode::Conservative,
                        t!("disk_analysis.dup_read_conservative").to_string(),
                    );
                    ui.selectable_value(
                        &mut state.duplicates.read_mode,
                        DuplicateReadMode::Parallel,
                        t!("disk_analysis.dup_read_parallel").to_string(),
                    );
                })
                .response
                .on_hover_text(t!("disk_analysis.dup_read_hint").to_string());
        });
    });
    ui.horizontal_wrapped(|ui| {
        ui.label(t!("disk_analysis.dup_min_size").to_string());
        ui.add_enabled(
            !running,
            egui::TextEdit::singleline(&mut state.duplicates.min_size_text).desired_width(90.0),
        );
        ui.label(t!("disk_analysis.unit_bytes").to_string());
        let valid_min_size = state.duplicates.min_size().is_some();
        if !running && !valid_min_size {
            ui.label(
                egui::RichText::new(t!("disk_analysis.dup_invalid_min_size").to_string())
                    .color(ui.visuals().error_fg_color),
            );
        }

        // Capture drive/model/subtree at start time (plan section 7).
        let root = state.current_root().unwrap_or(model.root);
        if ui
            .add_enabled(
                valid_min_size,
                egui::Button::new(t!("disk_analysis.dup_start").to_string()),
            )
            .clicked()
        {
            state.duplicates.start(model.clone(), root);
            ui.ctx().request_repaint();
        }

        if state.duplicates.report.is_none() {
            phase_label(ui, state.duplicates.phase);
        }
    });
}

fn render_report(
    state: &mut DiskAnalysisState,
    ui: &mut egui::Ui,
    model: &Arc<DiskAnalysisModel>,
    report: &Arc<DuplicateReport>,
) {
    let stats = report.stats;
    ui.horizontal_wrapped(|ui| {
        ui.label(
            egui::RichText::new(
                t!(
                    "disk_analysis.dup_summary",
                    groups = report.groups.len(),
                    recoverable = format_size(report.total_recoverable)
                )
                .to_string(),
            )
            .strong(),
        );
    });
    details::warnings(
        ui,
        stats.inaccessible,
        stats.changed_during_read,
        stats.skipped_unavailable,
    );

    egui::ScrollArea::vertical()
        .id_salt("disk_analysis_dup_report_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            details::render(ui, &stats, None, true);
            if report.groups.is_empty() {
                ui.label(t!("disk_analysis.dup_none").to_string());
                return;
            }
            for (group_idx, group) in report.groups.iter().enumerate() {
                let header_id = egui::Id::new("dup_group")
                    .with(group.logical_size)
                    .with(group.members.first().map(|m| m.idx).unwrap_or(0));
                let header = format!(
                    "{} \u{00D7} {} \u{2014} +{}{}",
                    group.members.len(),
                    format_size(group.logical_size),
                    format_size(group.recoverable),
                    if group.has_hardlinks {
                        " \u{1F517}"
                    } else {
                        ""
                    },
                );
                egui::CollapsingHeader::new(egui::RichText::new(header).strong())
                    .id_salt(header_id)
                    .show(ui, |ui| {
                        if ui
                            .button(t!("disk_analysis.dup_compare").to_string())
                            .clicked()
                        {
                            state.duplicates.review.open_group(report, group_idx);
                        }
                        let total_w = ui.available_width();
                        for member in &group.members {
                            let resp = ui.allocate_response(
                                egui::vec2(total_w, ROW_HEIGHT),
                                egui::Sense::click(),
                            );
                            if resp.hovered() {
                                ui.painter().rect_filled(
                                    resp.rect,
                                    3.0,
                                    ui.visuals().widgets.hovered.bg_fill,
                                );
                            }
                            paint_member_row(
                                ui.painter(),
                                resp.rect,
                                model,
                                member.idx,
                                total_w,
                                analyzer_text_color(ui),
                                weak_text(ui),
                            );
                            if resp.clicked() {
                                // Selecting a member highlights it in the treemap.
                                let idx = member.idx;
                                if model.nodes[idx as usize].is_dir {
                                    state.navigate_to(idx);
                                } else {
                                    state.reveal_file(idx);
                                }
                            }
                            attach_row_menu(state, ui, &resp, member.idx, true);
                        }
                    });
            }
        });
}

/// Member row inside a group: path + allocated size (physical footprint).
#[allow(clippy::too_many_arguments)]
fn paint_member_row(
    painter: &egui::Painter,
    rect: egui::Rect,
    model: &Arc<DiskAnalysisModel>,
    idx: u32,
    total_w: f32,
    text_color: egui::Color32,
    weak: egui::Color32,
) {
    let node = &model.nodes[idx as usize];
    let mut x = rect.left();
    let name_w = (total_w * 0.35).clamp(140.0, 320.0);
    let alloc_w = 90.0;

    paint_cell(painter, rect, &mut x, name_w, &node.name, false, text_color);
    paint_cell(
        painter,
        rect,
        &mut x,
        total_w - name_w - alloc_w,
        &model.path_of(idx),
        false,
        weak,
    );
    paint_cell(
        painter,
        rect,
        &mut x,
        alloc_w,
        &format_size(node.allocated_size),
        true,
        weak,
    );
}
