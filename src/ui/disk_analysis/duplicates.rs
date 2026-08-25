//! "Duplicates" tab: manual duplicate detection UI (plan section 7).
//!
//! The scan only starts on an explicit button press; progress and the final
//! report render from [`DuplicateSession`]. Report actions are limited to
//! **Open in main app** and **Copy path** — no deletion here.

use super::{
    analyzer_text_color, attach_row_menu, largest_items::weak_text, paint_cell,
    panel_secondary_text_color, ROW_HEIGHT,
};
use crate::app::disk_analysis_duplicates::{DuplicatePhase, DuplicateReport};
use crate::app::disk_analysis_model::DiskAnalysisModel;
use crate::app::disk_analysis_state::DiskAnalysisState;
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

    render_controls(state, ui, &model);

    if state.duplicates.is_running() {
        ui.add_space(6.0);
        render_progress(state, ui, &model);
        return;
    }

    match state.duplicates.report.clone() {
        Some(report) => {
            ui.add_space(6.0);
            render_report(state, ui, &model, &report);
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
    ui.horizontal(|ui| {
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
                    .small()
                    .color(ui.visuals().error_fg_color),
            );
        }

        // Capture drive/model/subtree at start time (plan section 7).
        let root = state.current_root().unwrap_or(model.root);
        if running {
            if ui
                .button(t!("disk_analysis.dup_cancel").to_string())
                .clicked()
            {
                state.duplicates.cancel();
                ui.ctx().request_repaint();
            }
        } else if ui
            .add_enabled(
                valid_min_size,
                egui::Button::new(t!("disk_analysis.dup_start").to_string()),
            )
            .clicked()
        {
            state.duplicates.start(model.clone(), root);
            ui.ctx().request_repaint();
        }

        phase_label(ui, state.duplicates.phase);
    });
}

fn phase_label(ui: &mut egui::Ui, phase: DuplicatePhase) {
    let text = match phase {
        DuplicatePhase::Collecting => t!("disk_analysis.dup_phase_collecting").to_string(),
        DuplicatePhase::Hashing => t!("disk_analysis.dup_phase_hashing").to_string(),
        DuplicatePhase::Finalizing => t!("disk_analysis.dup_phase_finalizing").to_string(),
        DuplicatePhase::Partial => t!("disk_analysis.dup_phase_partial").to_string(),
        DuplicatePhase::Cancelled => t!("disk_analysis.dup_phase_cancelled").to_string(),
        DuplicatePhase::Failed => t!("disk_analysis.dup_phase_failed").to_string(),
        DuplicatePhase::Complete | DuplicatePhase::Idle => return,
    };
    ui.label(
        egui::RichText::new(text)
            .small()
            .color(panel_secondary_text_color(ui)),
    );
}

fn render_progress(
    state: &mut DiskAnalysisState,
    ui: &mut egui::Ui,
    model: &Arc<DiskAnalysisModel>,
) {
    let progress = state.duplicates.progress.clone();
    let fraction = if progress.total_candidates > 0 {
        (progress.processed_files as f32 / progress.total_candidates as f32).clamp(0.02, 1.0)
    } else {
        0.02
    };
    ui.add(egui::ProgressBar::new(fraction).show_percentage());
    ui.horizontal_wrapped(|ui| {
        ui.label(format!(
            "{} {} / {}",
            t!("disk_analysis.dup_processed"),
            progress.processed_files,
            progress.total_candidates
        ));
        ui.separator();
        ui.label(format_size(progress.hashed_bytes));
        if let Some(idx) = progress.current_idx {
            ui.separator();
            ui.label(
                egui::RichText::new(model.path_of(idx))
                    .small()
                    .color(panel_secondary_text_color(ui)),
            );
        }
    });
    counters_line(
        ui,
        progress.inaccessible,
        progress.changed_during_read,
        progress.skipped_unavailable,
        progress.skipped_by_limit,
    );
}

fn counters_line(
    ui: &mut egui::Ui,
    inaccessible: u64,
    changed: u64,
    skipped: u64,
    skipped_by_limit: u64,
) {
    ui.horizontal_wrapped(|ui| {
        if inaccessible > 0 {
            ui.label(
                egui::RichText::new(
                    t!("disk_analysis.dup_inaccessible", count = inaccessible).to_string(),
                )
                .small(),
            );
        }
        if changed > 0 {
            ui.label(
                egui::RichText::new(t!("disk_analysis.dup_changed", count = changed).to_string())
                    .small(),
            );
        }
        if skipped > 0 {
            ui.label(
                egui::RichText::new(t!("disk_analysis.dup_skipped", count = skipped).to_string())
                    .small(),
            );
        }
        if skipped_by_limit > 0 {
            ui.label(
                egui::RichText::new(
                    t!("disk_analysis.dup_skipped_limit", count = skipped_by_limit).to_string(),
                )
                .small(),
            );
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
        if stats.hardlink_only_groups > 0 {
            ui.label(
                egui::RichText::new(
                    t!(
                        "disk_analysis.dup_hardlink_note",
                        groups = stats.hardlink_only_groups
                    )
                    .to_string(),
                )
                .small()
                .color(panel_secondary_text_color(ui)),
            );
        }
    });
    ui.label(
        egui::RichText::new(
            t!(
                "disk_analysis.dup_report_stats",
                processed = stats.processed_files,
                candidates = stats.total_candidates,
                bytes = format_size(stats.hashed_bytes)
            )
            .to_string(),
        )
        .small()
        .color(panel_secondary_text_color(ui)),
    );
    counters_line(
        ui,
        stats.inaccessible,
        stats.changed_during_read,
        stats.skipped_unavailable,
        stats.skipped_by_limit,
    );

    if report.groups.is_empty() {
        ui.label(t!("disk_analysis.dup_none").to_string());
        return;
    }

    egui::ScrollArea::vertical()
        .id_salt("disk_analysis_dup_report_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for group in &report.groups {
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
