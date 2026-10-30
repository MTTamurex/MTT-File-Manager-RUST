//! A comparison window. Filesystem work belongs to background workers,
//! never to these widgets.
use super::*;
use crate::app::disk_analysis_duplicates::{DuplicateGroup, DuplicateReview};

mod card;

pub(super) fn render(
    state: &mut DiskAnalysisState,
    parent: &mut egui::Ui,
    model: &Arc<DiskAnalysisModel>,
    report: &Arc<DuplicateReport>,
) {
    let Some(group_idx) = state.duplicates.review.active_group else {
        return;
    };
    let Some(group) = report.groups.get(group_idx) else {
        state.duplicates.review.close();
        return;
    };
    state.duplicates.review.queue_visible(model);
    let mut open = true;
    let style = parent.style().clone();
    egui::Window::new(t!("disk_analysis.dup_compare").to_string())
        .id(egui::Id::new("duplicate_comparison_window"))
        .open(&mut open)
        .default_size([1040.0, 720.0])
        .min_width(340.0)
        .resizable(true)
        .show(parent.ctx(), |ui| {
            ui.set_style(style);
            ui.spacing_mut().item_spacing = egui::vec2(12.0, 10.0);
            ui.spacing_mut().button_padding = egui::vec2(10.0, 6.0);
            ui.add(egui::Label::new(t!("disk_analysis.dup_review_intro").to_string()).wrap());
            ui.horizontal_wrapped(|ui| {
                if ui
                    .button(t!("disk_analysis.dup_review_refresh").to_string())
                    .clicked()
                {
                    state.duplicates.review.refresh();
                }
            });
            ui.separator();
            egui::ScrollArea::vertical()
                .id_salt("duplicate_comparison_cards")
                .show(ui, |ui| {
                    if ui.available_width() >= 820.0 {
                        ui.columns(2, |columns| {
                            for (slot, ui) in columns.iter_mut().enumerate() {
                                card::render(
                                    ui,
                                    &mut state.duplicates.review,
                                    &mut state.duplicates.deletion,
                                    slot,
                                    report,
                                    group,
                                    model,
                                );
                            }
                        });
                    } else {
                        for slot in 0..2 {
                            if slot > 0 {
                                ui.add_space(8.0);
                            }
                            card::render(
                                ui,
                                &mut state.duplicates.review,
                                &mut state.duplicates.deletion,
                                slot,
                                report,
                                group,
                                model,
                            );
                        }
                    }
                    ui.add_space(6.0);
                    ui.separator();
                    ui.add(
                        egui::Label::new(t!("disk_analysis.dup_review_freshness").to_string())
                            .wrap(),
                    );
                    ui.add(
                        egui::Label::new(
                            t!("disk_analysis.dup_review_allocation_note").to_string(),
                        )
                        .wrap(),
                    );
                    if group.has_hardlinks {
                        ui.add(
                            egui::Label::new(t!("disk_analysis.dup_review_hardlinks").to_string())
                                .wrap(),
                        );
                    }
                });
        });
    if !open {
        state.duplicates.review.close();
    }
    if state.duplicates.review.is_pending() {
        parent
            .ctx()
            .request_repaint_after(std::time::Duration::from_millis(50));
    }
}

fn selector(
    ui: &mut egui::Ui,
    review: &mut DuplicateReview,
    slot: usize,
    group: &DuplicateGroup,
    model: &DiskAnalysisModel,
) -> Option<u32> {
    let idx = review.entries[slot].as_ref()?.idx;
    let position = group.members.iter().position(|m| m.idx == idx)?;
    let mut selected = idx;
    egui::ComboBox::from_id_salt(("duplicate_review_copy", slot))
        .selected_text(
            t!(
                "disk_analysis.dup_review_copy_number",
                number = position + 1,
                total = group.members.len()
            )
            .to_string(),
        )
        .width(ui.available_width().min(300.0))
        .show_ui(ui, |ui| {
            egui::ScrollArea::vertical().max_height(200.0).show_rows(
                ui,
                ROW_HEIGHT,
                group.members.len(),
                |ui, range| {
                    for position in range {
                        let member = &group.members[position];
                        let label = format!(
                            "{} · {}",
                            position + 1,
                            model.nodes[member.idx as usize].name
                        );
                        ui.selectable_value(&mut selected, member.idx, label)
                            .on_hover_text(model.path_of(member.idx));
                    }
                },
            );
        });
    if selected != idx {
        review.select_copy(slot, selected);
    }
    review.entries[slot].as_ref().map(|e| e.idx)
}
