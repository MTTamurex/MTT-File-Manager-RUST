//! Human-readable coverage warning, separate from optional diagnostic counters.
use super::*;
use crate::app::disk_analysis_duplicates::DuplicateStats;

fn unchecked(inaccessible: u64, changed: u64, unavailable: u64) -> u64 {
    inaccessible
        .saturating_add(changed)
        .saturating_add(unavailable)
}

pub(super) fn warnings(ui: &mut egui::Ui, inaccessible: u64, changed: u64, unavailable: u64) {
    let count = unchecked(inaccessible, changed, unavailable);
    if count == 0 {
        return;
    }
    egui::CollapsingHeader::new(
        egui::RichText::new(t!("disk_analysis.dup_unchecked_warning", count = count).to_string())
            .color(ui.visuals().warn_fg_color),
    )
    .id_salt("duplicate_coverage_warning")
    .default_open(false)
    .show(ui, |ui| {
        for (count, key) in [
            (inaccessible, "disk_analysis.dup_inaccessible"),
            (changed, "disk_analysis.dup_changed"),
            (unavailable, "disk_analysis.dup_skipped"),
        ] {
            if count > 0 {
                ui.label(t!(key, count = count).to_string());
            }
        }
    });
}

pub(super) fn render(
    ui: &mut egui::Ui,
    stats: &DuplicateStats,
    current: Option<(&DiskAnalysisModel, u32)>,
    filter_ready: bool,
) {
    egui::CollapsingHeader::new(t!("disk_analysis.dup_search_details").to_string())
        .id_salt("duplicate_search_details")
        .default_open(false)
        .show(ui, |ui| {
            ui.label(
                t!(
                    "disk_analysis.dup_report_stats",
                    processed = stats.processed_files,
                    candidates = stats.total_candidates,
                    bytes = format_size(stats.hashed_bytes)
                )
                .to_string(),
            );
            if let Some((model, idx)) = current {
                ui.label(model.path_of(idx));
            }
            if filter_ready {
                ui.label(
                    t!(
                        "disk_analysis.dup_filter_stats",
                        rejected = stats.partial_rejected,
                        full = stats.full_candidates
                    )
                    .to_string(),
                );
            }
            if stats.skipped_by_limit > 0 {
                ui.label(
                    t!(
                        "disk_analysis.dup_skipped_limit",
                        count = stats.skipped_by_limit
                    )
                    .to_string(),
                );
            }
            if stats.hardlink_only_groups > 0 {
                ui.label(
                    t!(
                        "disk_analysis.dup_hardlink_note",
                        groups = stats.hardlink_only_groups
                    )
                    .to_string(),
                );
            }
        });
}

#[cfg(test)]
mod tests;
