use super::*;
use crate::infrastructure::copy_engine::Outcome;
use std::path::PathBuf;

#[test]
fn badge_reserves_a_separate_area_on_wide_and_narrow_windows() {
    let wide = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 24.0));
    let (status, badge) = rects(wide, 180.0, 24.0, false);
    assert!(status.right() <= badge.left());
    assert_eq!(status.width(), 1020.0);
    let narrow = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(480.0, 72.0));
    let (status, badge) = rects(narrow, 180.0, 24.0, true);
    assert_eq!(status.width(), narrow.width());
    assert_eq!(status.height(), 48.0);
    assert!(status.bottom() <= badge.top());
    let (status, badge) = rects(narrow, 0.0, 24.0, false);
    assert_eq!(status, narrow);
    assert_eq!(badge, egui::Rect::NOTHING);
}

#[test]
fn summary_weights_only_active_jobs_and_keeps_terminal_results_accessible() {
    let runtime = CopyRuntime::default();
    assert!(CopyIndicator::from_runtime(&runtime).is_none());
    let first = runtime.start(vec![PathBuf::from("file.txt")], PathBuf::from("target"));
    let second = runtime.start(vec![PathBuf::from("other.txt")], PathBuf::from("target"));
    first
        .operation
        .progress
        .total_bytes
        .store(100, Ordering::Relaxed);
    first
        .operation
        .progress
        .bytes_copied
        .store(50, Ordering::Relaxed);
    first
        .operation
        .progress
        .copying
        .store(true, Ordering::Relaxed);
    second
        .operation
        .progress
        .total_bytes
        .store(300, Ordering::Relaxed);
    second
        .operation
        .progress
        .copying
        .store(true, Ordering::Relaxed);
    let summary = CopyIndicator::from_runtime(&runtime).unwrap();
    assert_eq!(summary.count, 2);
    assert_eq!(summary.active, 2);
    assert_eq!(summary.fraction, Some(0.125));

    let mut panel = CopyPanelState::default();
    panel.minimize();
    first.finish(Outcome {
        kind: OutcomeKind::Completed,
        message: None,
    });
    let summary = CopyIndicator::from_runtime(&runtime).unwrap();
    assert_eq!(summary.active, 1);
    assert_eq!(summary.finished, 1);
    assert_eq!(summary.fraction, Some(0.0));
    second.finish(Outcome {
        kind: OutcomeKind::Failed,
        message: Some("Copy error".into()),
    });
    let summary = CopyIndicator::from_runtime(&runtime).unwrap();
    assert_eq!(summary.active, 0);
    assert_eq!(summary.finished, 2);
    assert_eq!(summary.failed, 1);
    assert_eq!(summary.fraction, None);
    assert!(panel.minimized); // Finishing or failing must not restore the panel.
    for (id, _) in runtime.jobs() {
        runtime.dismiss(id);
    }
    assert!(CopyIndicator::from_runtime(&runtime).is_none());
}

#[test]
fn preparing_and_empty_file_progress_do_not_divide_by_zero() {
    let runtime = CopyRuntime::default();
    let job = runtime.start(vec![PathBuf::from("file.txt")], PathBuf::from("target"));
    assert_eq!(
        CopyIndicator::from_runtime(&runtime).unwrap().fraction,
        None
    );
    job.operation
        .progress
        .copying
        .store(true, Ordering::Relaxed);
    assert_eq!(
        CopyIndicator::from_runtime(&runtime).unwrap().fraction,
        None
    );
    job.operation
        .progress
        .total_files
        .store(4, Ordering::Relaxed);
    job.operation
        .progress
        .files_copied
        .store(1, Ordering::Relaxed);
    assert_eq!(
        CopyIndicator::from_runtime(&runtime).unwrap().fraction,
        Some(0.25)
    );
    job.operation.progress.pause();
    assert_eq!(
        CopyIndicator::from_runtime(&runtime).unwrap().fraction,
        Some(0.25)
    );
}

#[test]
fn clicking_indicator_restores_the_panel_without_changing_the_copy() {
    let runtime = CopyRuntime::default();
    let job = runtime.start(vec![PathBuf::from("file.txt")], PathBuf::from("target"));
    let summary = CopyIndicator::from_runtime(&runtime).unwrap();
    let ctx = egui::Context::default();
    let mut panel = CopyPanelState::default();
    panel.minimize();
    let mut draw = |events: Vec<egui::Event>| {
        let mut rect = egui::Rect::NOTHING;
        let _ = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 80.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                ui.set_width(200.0);
                rect = render(ui, &summary, &mut panel).rect;
            },
        );
        rect
    };
    draw(Vec::new());
    let rect = draw(Vec::new());
    let position = rect.center();
    let button = |pressed| egui::Event::PointerButton {
        pos: position,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    draw(vec![egui::Event::PointerMoved(position), button(true)]);
    draw(vec![button(false)]);
    assert!(!panel.minimized);
    assert!(!job.operation.progress.is_cancelled());
    assert_eq!(
        job.operation.progress.pause_state(),
        mtt_copy_engine::PauseState::Running
    );
    assert!(job.operation.outcome().is_none());
}
