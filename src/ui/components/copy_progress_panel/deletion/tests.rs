use super::*;
use crate::app::copy_panel_state::CopyPanelState;
use crate::infrastructure::copy_engine::{CopyRuntime, Outcome};

fn texts(shape: &egui::epaint::Shape, result: &mut Vec<String>) {
    match shape {
        egui::epaint::Shape::Text(text) => result.push(text.galley.job.text.clone()),
        egui::epaint::Shape::Vec(shapes) => {
            for shape in shapes {
                texts(shape, result);
            }
        }
        _ => {}
    }
}

#[test]
fn deletion_uses_shared_window_without_copy_units_or_pause_controls() {
    for scale in [1.0, 2.0] {
        let ctx = egui::Context::default();
        crate::ui::interface_scale::apply(&ctx, scale);
        let runtime = CopyRuntime::default();
        let job = runtime.start_delete(vec![PathBuf::from("source/folder")], true);
        job.operation
            .deletion
            .as_ref()
            .unwrap()
            .removed
            .store(12, Ordering::Relaxed);
        let mut panel = CopyPanelState::default();
        let mut labels = Vec::new();
        for _ in 0..8 {
            labels.clear();
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200.0, 1000.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    super::super::window::show(ui.ctx(), &runtime.jobs(), &mut panel);
                },
            );
            for shape in output.shapes {
                texts(&shape.shape, &mut labels);
            }
        }
        assert!(labels
            .iter()
            .any(|label| label == &t!("delete_engine.deleting")));
        assert!(labels
            .iter()
            .any(|label| label == &t!("copy_engine.cancel")));
        assert!(!labels
            .iter()
            .any(|label| label.contains("MiB") || label == &t!("copy_engine.pause")));
        assert!(!job.operation.progress.is_cancelled());
        job.finish(Outcome {
            kind: OutcomeKind::Cancelled,
            message: Some(t!("delete_engine.cancelled").to_string()),
        });
        let elapsed = job.operation.elapsed();
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            card(ui, &job.operation, &[]);
        });
        labels.clear();
        for shape in output.shapes {
            texts(&shape.shape, &mut labels);
        }
        assert!(labels
            .iter()
            .any(|label| label == &t!("copy_engine.dismiss")));
        assert_eq!(job.operation.elapsed(), elapsed);
    }
}

#[test]
fn mixed_title_keeps_removed_items_separate_from_copied_bytes() {
    let runtime = CopyRuntime::default();
    let deletion = runtime.start_delete(vec![PathBuf::from("source/folder")], false);
    deletion
        .operation
        .deletion
        .as_ref()
        .unwrap()
        .removed
        .store(7, Ordering::Relaxed);
    let copy = runtime.start(
        vec![PathBuf::from("source/file.txt")],
        PathBuf::from("target"),
    );
    copy.operation
        .progress
        .copying
        .store(true, Ordering::Relaxed);
    copy.operation
        .progress
        .total_bytes
        .store(100, Ordering::Relaxed);
    copy.operation
        .progress
        .bytes_copied
        .store(25, Ordering::Relaxed);
    let label =
        super::super::summary::Summary::from_jobs(&runtime.jobs(), &CopyPanelState::default())
            .unwrap()
            .label();
    assert!(label.starts_with("25%"));
    assert!(label.contains(&t!("delete_engine.header", count = 7).to_string()));
    assert!(
        super::super::indicator::CopyIndicator::from_runtime(&runtime)
            .unwrap()
            .fraction
            .is_none()
    );
}
