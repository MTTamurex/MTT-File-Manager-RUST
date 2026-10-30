use super::*;
use crate::app::disk_analysis_duplicates::{CopyStatus, DuplicateDeletion, PreviewStatus};
use crate::infrastructure::windows::formatting::format_date;

fn status_text(status: CopyStatus) -> String {
    match status {
        CopyStatus::Current => t!("disk_analysis.dup_review_current"),
        CopyStatus::Changed => t!("disk_analysis.dup_review_changed"),
        CopyStatus::Missing => t!("disk_analysis.dup_review_missing"),
        CopyStatus::Unavailable => t!("disk_analysis.dup_review_offline"),
        CopyStatus::Inaccessible => t!("disk_analysis.dup_review_inaccessible"),
    }
    .to_string()
}

fn date_label(date: Option<u64>) -> String {
    date.map(format_date)
        .unwrap_or_else(|| t!("disk_analysis.dup_review_unknown_date").to_string())
}

pub(super) fn render(
    ui: &mut egui::Ui,
    review: &mut DuplicateReview,
    deletion: &mut DuplicateDeletion,
    slot: usize,
    report: &Arc<DuplicateReport>,
    model: &DiskAnalysisModel,
    hwnd: isize,
) {
    ui.push_id(slot, |ui| {
        egui::Frame::group(ui.style())
            .inner_margin(14)
            .show(ui, |ui| {
                let Some(group) = review.active_group.and_then(|g| report.groups.get(g)) else {
                    return;
                };
                let Some(idx) = selector(ui, review, slot, group, model) else {
                    return;
                };
                let Some(member) = group.members.iter().find(|m| m.idx == idx) else {
                    return;
                };
                let path = model.path_of(idx);
                ui.add_space(2.0);
                ui.label(egui::RichText::new(&model.nodes[idx as usize].name).strong());
                ui.add(egui::Label::new(&path).wrap());
                if group.members.iter().any(|other| {
                    other.idx != idx && other.physical_identity() == member.physical_identity()
                }) {
                    ui.label(t!("disk_analysis.dup_review_hardlinks").to_string());
                }
                ui.add_space(2.0);
                ui.separator();
                let entry = review.entries[slot].as_mut().unwrap();
                let mut current = false;
                if let Some(info) = &mut entry.info {
                    current = info.status == CopyStatus::Current;
                    let text = egui::RichText::new(status_text(info.status));
                    ui.label(if current {
                        text
                    } else {
                        text.color(ui.visuals().warn_fg_color)
                    });
                    egui::Grid::new("copy_metadata")
                        .num_columns(2)
                        .spacing([18.0, 10.0])
                        .show(ui, |ui| {
                            ui.label(t!("disk_analysis.logical_size_short").to_string());
                            ui.label(info.size.map(format_size).unwrap_or_else(|| {
                                t!("disk_analysis.dup_review_unknown_date").to_string()
                            }));
                            ui.end_row();
                            ui.label(t!("disk_analysis.allocated_size").to_string());
                            ui.label(format_size(member.allocated_size));
                            ui.end_row();
                            ui.label(t!("disk_analysis.dup_review_created").to_string());
                            ui.label(date_label(info.created));
                            ui.end_row();
                            ui.label(t!("disk_analysis.dup_review_modified").to_string());
                            ui.label(date_label(info.modified));
                            ui.end_row();
                        });
                    if let Some(image) = info.image.take() {
                        let image =
                            egui::ColorImage::from_rgba_unmultiplied(image.size, &image.rgba);
                        entry.texture = Some(ui.ctx().load_texture(
                            format!("duplicate-copy-{idx}"),
                            image,
                            egui::TextureOptions::LINEAR,
                        ));
                    }
                    if let Some(texture) = &entry.texture {
                        let size = texture.size_vec2();
                        let factor = (ui.available_width().max(1.0) / size.x).min(1.0);
                        ui.add(egui::Image::new(texture).fit_to_exact_size(size * factor));
                    } else {
                        match info.preview_status {
                            PreviewStatus::TooLarge => {
                                ui.label(t!("disk_analysis.dup_review_preview_large").to_string());
                            }
                            PreviewStatus::Unavailable if current => {
                                ui.label(
                                    t!("disk_analysis.dup_review_preview_unavailable").to_string(),
                                );
                            }
                            _ => {}
                        }
                    }
                } else {
                    ui.spinner();
                    ui.label(t!("disk_analysis.dup_review_loading").to_string());
                }
                ui.add_space(4.0);
                ui.separator();
                let other_current = review.entries.iter().flatten().any(|e| {
                    e.idx != idx
                        && review.current(e.idx)
                        && group.members.iter().any(|other| {
                            other.idx == e.idx
                                && other.physical_identity() != member.physical_identity()
                        })
                });
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(
                            current && other_current && !deletion.is_open(),
                            egui::Button::new(t!("disk_analysis.dup_delete_file").to_string()),
                        )
                        .clicked()
                    {
                        deletion.start(report, review, model, idx, hwnd);
                    }
                    if ui
                        .add_enabled(
                            current,
                            egui::Button::new(t!("disk_analysis.open_in_main_app").to_string()),
                        )
                        .clicked()
                    {
                        crate::disk_analyzer::open_in_main::open_path_in_main_app(&path, false);
                    }
                    if ui
                        .button(t!("disk_analysis.copy_path").to_string())
                        .clicked()
                    {
                        let _ = crate::application::file_operations::copy_path_to_clipboard(
                            std::path::Path::new(&path),
                        );
                    }
                });
            });
    });
}
