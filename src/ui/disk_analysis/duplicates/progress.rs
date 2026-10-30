//! Stage-specific progress; technical counters are collapsed by default.
use super::*;
use crate::app::disk_analysis_duplicates::{DuplicatePhase, DuplicateProgress, DuplicateStats};

pub(super) fn phase_label(ui: &mut egui::Ui, phase: DuplicatePhase) {
    let text = match phase {
        DuplicatePhase::Collecting => t!("disk_analysis.dup_phase_collecting"),
        DuplicatePhase::Comparing => t!("disk_analysis.dup_phase_comparing"),
        DuplicatePhase::Hashing => t!("disk_analysis.dup_phase_hashing"),
        DuplicatePhase::Finalizing => t!("disk_analysis.dup_phase_finalizing"),
        DuplicatePhase::Partial => t!("disk_analysis.dup_phase_partial"),
        DuplicatePhase::Cancelled => t!("disk_analysis.dup_phase_cancelled"),
        DuplicatePhase::Failed => t!("disk_analysis.dup_phase_failed"),
        DuplicatePhase::Complete | DuplicatePhase::Idle => return,
    };
    ui.label(text.to_string());
}

fn fraction(phase: DuplicatePhase, progress: &DuplicateProgress) -> Option<f32> {
    if matches!(phase, DuplicatePhase::Comparing | DuplicatePhase::Hashing)
        && progress.phase_total > 0
    {
        Some((progress.phase_processed as f32 / progress.phase_total as f32).clamp(0.0, 1.0))
    } else {
        None
    }
}

pub(super) fn render_progress(
    state: &mut DiskAnalysisState,
    ui: &mut egui::Ui,
    model: &Arc<DiskAnalysisModel>,
) {
    let progress = &state.duplicates.progress;
    if let Some(fraction) = fraction(state.duplicates.phase, progress) {
        ui.add(egui::ProgressBar::new(fraction).show_percentage());
        ui.label(
            t!(
                "disk_analysis.dup_stage_progress",
                processed = progress.phase_processed,
                total = progress.phase_total
            )
            .to_string(),
        );
    } else {
        ui.spinner();
    }
    details::warnings(
        ui,
        progress.inaccessible,
        progress.changed_during_read,
        progress.skipped_unavailable,
    );
    let stats = DuplicateStats {
        processed_files: progress.processed_files,
        total_candidates: progress.total_candidates,
        hashed_bytes: progress.hashed_bytes,
        partial_rejected: progress.partial_rejected,
        full_candidates: progress.full_candidates,
        skipped_by_limit: progress.skipped_by_limit,
        ..Default::default()
    };
    egui::ScrollArea::vertical()
        .id_salt("duplicate_live_details")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            details::render(
                ui,
                &stats,
                progress.current_idx.map(|idx| (model.as_ref(), idx)),
                matches!(
                    state.duplicates.phase,
                    DuplicatePhase::Hashing | DuplicatePhase::Finalizing
                ),
            );
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn percentage_tracks_only_the_current_stage_without_artificial_minimum() {
        let mut progress = DuplicateProgress {
            total_candidates: 100,
            processed_files: 90,
            phase_total: 10,
            phase_processed: 0,
            ..Default::default()
        };
        assert_eq!(fraction(DuplicatePhase::Hashing, &progress), Some(0.0));
        progress.phase_processed = 5;
        assert_eq!(fraction(DuplicatePhase::Hashing, &progress), Some(0.5));
        assert_eq!(fraction(DuplicatePhase::Collecting, &progress), None);
        assert_eq!(fraction(DuplicatePhase::Finalizing, &progress), None);
        progress.phase_total = 0;
        assert_eq!(fraction(DuplicatePhase::Comparing, &progress), None);
    }
}
