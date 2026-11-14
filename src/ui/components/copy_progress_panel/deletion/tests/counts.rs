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
