use super::*;

#[test]
fn completed_recycle_permanent_and_mixed_operations_show_the_correct_counts_and_title() {
    for scale in [1.0, 2.0] {
        for mode in 0..3 {
            let ctx = egui::Context::default();
            crate::ui::interface_scale::apply(&ctx, scale);
            let runtime = CopyRuntime::default();
            let job = runtime.start_delete(vec![PathBuf::from("source/file.txt")], mode == 1);
            let progress = job.operation.deletion.as_ref().unwrap();
            if mode != 1 {
                progress.recycled();
            }
            if mode != 0 {
                progress.removed();
            }
            if mode == 2 {
                progress
                    .used_permanent_fallback
                    .store(true, Ordering::Relaxed);
            }
            job.finish(Outcome {
                kind: OutcomeKind::Completed,
                message: None,
            });
            let mut panel = CopyPanelState::default();
            let mut labels = Vec::new();
            for _ in 0..8 {
                let output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1200.0, 1000.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        super::super::super::window::show(ui.ctx(), &runtime.jobs(), &mut panel);
                    },
                );
                labels.clear();
                for shape in output.shapes {
                    texts(&shape.shape, &mut labels);
                }
            }
            assert_eq!(
                labels
                    .iter()
                    .any(|label| label == &t!("delete_engine.recycled_count")),
                mode != 1
            );
            assert_eq!(
                labels
                    .iter()
                    .any(|label| label == &t!("delete_engine.permanent_count")),
                mode != 0
            );
            let expected = match mode {
                0 => t!("delete_engine.header_recycled", count = 1),
                1 => t!("delete_engine.header_permanent", count = 1),
                _ => t!("delete_engine.header_mixed", recycled = 1, permanent = 1),
            };
            assert!(labels.iter().any(|label| label == &expected));
            assert_eq!(
                labels.iter().filter(|label| label.as_str() == "1").count(),
                if mode == 2 { 2 } else { 1 }
            );
            assert_eq!(
                progress.removed.load(Ordering::Relaxed),
                if mode == 2 { 2 } else { 1 }
            );
        }
    }
}

#[test]
fn terminal_deletion_errors_are_shown_once() {
    let context = egui::Context::default();
    let runtime = CopyRuntime::default();
    let job = runtime.start_delete(vec![PathBuf::from("source/file.txt")], true);
    let progress = job.operation.deletion.as_ref().unwrap();
    progress.total.store(1, Ordering::Relaxed);
    progress.failed();
    let failure_message = t!("delete_engine.failed", count = 1).to_string();
    job.finish(Outcome {
        kind: OutcomeKind::Failed,
        message: Some(failure_message.clone()),
    });

    let mut labels = Vec::new();
    let output = context.run_ui(egui::RawInput::default(), |ui| {
        super::super::card(ui, &job.operation, &[]);
    });
    for shape in output.shapes {
        texts(&shape.shape, &mut labels);
    }

    assert_eq!(
        labels
            .iter()
            .filter(|label| label.as_str() == failure_message.as_str())
            .count(),
        1
    );
}

#[test]
fn terminal_deletion_error_keeps_failure_count_when_message_is_different() {
    let context = egui::Context::default();
    let runtime = CopyRuntime::default();
    let job = runtime.start_delete(vec![PathBuf::from("source/file.txt")], true);
    let progress = job.operation.deletion.as_ref().unwrap();
    progress.failed();
    let failure_count = t!("delete_engine.failed", count = 1).to_string();
    job.finish(Outcome {
        kind: OutcomeKind::Failed,
        message: Some("operation failed".to_string()),
    });

    let mut labels = Vec::new();
    let output = context.run_ui(egui::RawInput::default(), |ui| {
        super::super::card(ui, &job.operation, &[]);
    });
    for shape in output.shapes {
        texts(&shape.shape, &mut labels);
    }

    assert!(labels.iter().any(|label| label == &failure_count));
    assert!(labels.iter().any(|label| label == "operation failed"));
}
