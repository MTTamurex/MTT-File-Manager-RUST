use super::*;

#[test]
fn cancelled_result_keeps_its_message_without_a_progress_bar_or_duplicated_status() {
    for scale in [1.0, 2.0] {
        let mut heights = Vec::new();
        for kind in [OutcomeKind::Completed, OutcomeKind::Cancelled] {
            let ctx = egui::Context::default();
            crate::ui::interface_scale::apply(&ctx, scale);
            let runtime = CopyRuntime::default();
            let job = runtime.start_delete(vec![PathBuf::from("source/file.txt")], false);
            job.finish(Outcome {
                kind,
                message: None,
            });
            let mut height = 0.0;
            let output = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.set_width(520.0);
                let before = ui.cursor().top();
                card(ui, &job.operation, &[]);
                height = ui.cursor().top() - before;
            });
            heights.push(height);
            if kind == OutcomeKind::Cancelled {
                let mut labels = Vec::new();
                for shape in output.shapes {
                    texts(&shape.shape, &mut labels);
                }
                assert_eq!(
                    labels
                        .iter()
                        .filter(|label| *label == &t!("delete_engine.cancelled_title"))
                        .count(),
                    1
                );
                assert!(labels
                    .iter()
                    .any(|label| label == &t!("copy_engine.dismiss")));
                // Keep the explanatory outcome text, independently of the bar.
                let message = t!("delete_engine.cancelled").to_string();
                job.finish(Outcome {
                    kind,
                    message: Some(message.clone()),
                });
                let output = ctx.run_ui(egui::RawInput::default(), |ui| {
                    card(ui, &job.operation, &[]);
                });
                labels.clear();
                for shape in output.shapes {
                    texts(&shape.shape, &mut labels);
                }
                assert!(labels.contains(&message));
            }
        }
        assert!(
            heights[0] - heights[1] >= 10.0,
            "Cancelled results must omit the progress bar at scale {scale}: {heights:?}"
        );
    }
}
