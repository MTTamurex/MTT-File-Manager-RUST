use super::*;

#[test]
fn elapsed_duration_formats_seconds_minutes_and_hours() {
    assert_eq!(format_duration(Duration::ZERO), "0 min 00 s");
    assert_eq!(format_duration(Duration::from_secs(65)), "1 min 05 s");
    assert_eq!(
        format_duration(Duration::from_secs(3665)),
        "1 h 01 min 05 s"
    );
}

#[test]
fn progress_handles_empty_files_rollback_and_completed_empty_trees() {
    assert_eq!(fraction(0, 0, 0, 0, false), 0.0);
    assert_eq!(fraction(0, 0, 1, 2, false), 0.5);
    assert_eq!(fraction(0, 0, 0, 0, true), 1.0);
    assert_eq!(fraction(120, 100, 1, 1, false), 1.0);
}

#[test]
fn auto_dismiss_removes_only_successful_copy_and_move_results_when_enabled() {
    use crate::infrastructure::copy_engine::{Outcome, OutcomeKind};
    use std::path::PathBuf;
    use std::sync::Arc;

    let runtime = CopyRuntime::default();
    let successful_copy = runtime.start(vec![PathBuf::from("source/copy.txt")], "target".into());
    successful_copy.finish(Outcome {
        kind: OutcomeKind::Completed,
        message: None,
    });
    let successful_move =
        runtime.start_move(vec![PathBuf::from("source/move.txt")], "target".into());
    successful_move.finish(Outcome {
        kind: OutcomeKind::Completed,
        message: None,
    });
    let successful_deletion = runtime.start_delete(vec![PathBuf::from("source/delete.txt")], false);
    successful_deletion.finish(Outcome {
        kind: OutcomeKind::Completed,
        message: None,
    });
    let failed_copy = runtime.start(vec![PathBuf::from("source/failed.txt")], "target".into());
    failed_copy.finish(Outcome {
        kind: OutcomeKind::Failed,
        message: None,
    });
    let cancelled_move =
        runtime.start_move(vec![PathBuf::from("source/cancelled.txt")], "target".into());
    cancelled_move.finish(Outcome {
        kind: OutcomeKind::Cancelled,
        message: None,
    });
    let active_copy = runtime.start(vec![PathBuf::from("source/active.txt")], "target".into());

    let mut jobs = runtime.jobs();
    dismiss_successful_transfers(&runtime, &mut jobs, false, false);
    assert_eq!(jobs.len(), 6);

    dismiss_successful_transfers(&runtime, &mut jobs, true, false);
    assert_eq!(jobs.len(), 4);
    for expected in [
        &successful_deletion.operation,
        &failed_copy.operation,
        &cancelled_move.operation,
        &active_copy.operation,
    ] {
        assert!(jobs
            .iter()
            .any(|(_, operation)| Arc::ptr_eq(operation, expected)));
    }
    assert!(!jobs
        .iter()
        .any(|(_, operation)| Arc::ptr_eq(operation, &successful_copy.operation)));
    assert!(!jobs
        .iter()
        .any(|(_, operation)| Arc::ptr_eq(operation, &successful_move.operation)));
}

#[test]
fn auto_dismiss_removes_only_successful_deletions_when_enabled() {
    use crate::infrastructure::copy_engine::{Outcome, OutcomeKind};
    use std::path::PathBuf;

    let runtime = CopyRuntime::default();
    let successful = runtime.start_delete(vec![PathBuf::from("source/delete.txt")], false);
    successful.finish(Outcome {
        kind: OutcomeKind::Completed,
        message: None,
    });
    let failed = runtime.start_delete(vec![PathBuf::from("source/failed.txt")], false);
    failed.finish(Outcome {
        kind: OutcomeKind::Failed,
        message: None,
    });
    let copy = runtime.start(vec![PathBuf::from("source/copy.txt")], "target".into());
    copy.finish(Outcome {
        kind: OutcomeKind::Completed,
        message: None,
    });

    let mut jobs = runtime.jobs();
    dismiss_successful_transfers(&runtime, &mut jobs, false, true);
    assert_eq!(jobs.len(), 2);
    assert!(jobs
        .iter()
        .all(|(_, operation)| !std::sync::Arc::ptr_eq(operation, &successful.operation)));
}

#[test]
fn cards_render_in_application_themes_without_changing_operation_state() {
    use crate::app::navigation_state::ThemeMode;
    use crate::infrastructure::copy_engine::{CopyRuntime, Outcome};
    use std::path::PathBuf;
    use std::sync::Arc;
    let runtime = Arc::new(CopyRuntime::default());
    let job = runtime.start(
        vec![PathBuf::from("source/file.txt")],
        PathBuf::from("target"),
    );
    let ctx = egui::Context::default();
    let mut metrics = TransferMetrics::default();
    for mode in [
        ThemeMode::Light,
        ThemeMode::Dark,
        ThemeMode::DarkDeep,
        ThemeMode::GitHubDark,
    ] {
        theme::apply_theme_visuals(&ctx, mode);
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            card(ui, &job.operation, &mut metrics);
        });
        assert!(!output.shapes.is_empty());
        assert!(!job.operation.progress.is_cancelled());
    }
    job.finish(Outcome {
        kind: OutcomeKind::Failed,
        message: Some("Copy error".into()),
    });
    let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
        card(ui, &job.operation, &mut metrics);
    });
    assert_eq!(job.operation.outcome().unwrap().kind, OutcomeKind::Failed);
}
