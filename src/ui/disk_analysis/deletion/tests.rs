use super::*;

#[test]
fn blocked_deletion_is_visible_and_can_be_dismissed_without_starting_an_operation() {
    let ctx = egui::Context::default();
    let mut state = DiskAnalysisState::new();
    state.duplicates.deletion.status = Some(DeleteStatus::Unsafe);
    let mut button = None;
    for _ in 0..3 {
        let output = ctx.run_ui(egui::RawInput::default(), |ui| render(&mut state, ui.ctx()));
        let mut warning_visible = false;
        for shape in output.shapes {
            if let egui::Shape::Text(text) = shape.shape {
                if text.galley.text() == t!("disk_analysis.dup_delete_unsafe").as_ref() {
                    warning_visible = true;
                }
                if text.galley.text() == t!("disk_analysis.dup_delete_close").as_ref() {
                    button = Some(text.pos + text.galley.size() * 0.5);
                }
            }
        }
        if button.is_some() {
            assert!(warning_visible);
        }
    }
    let pos = button.expect("close button visible");
    for pressed in [true, false] {
        let _ = ctx.run_ui(
            egui::RawInput {
                events: vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::default(),
                    },
                ],
                ..Default::default()
            },
            |ui| render(&mut state, ui.ctx()),
        );
    }
    assert!(!state.duplicates.deletion.is_open());
    assert!(!state.duplicates.deletion.is_running());
}

#[test]
fn no_deletion_dialog_without_an_explicit_request_or_result() {
    let ctx = egui::Context::default();
    let mut state = DiskAnalysisState::new();
    let output = ctx.run_ui(egui::RawInput::default(), |ui| render(&mut state, ui.ctx()));
    assert!(!output
        .shapes
        .iter()
        .any(|s| matches!(&s.shape, egui::Shape::Text(_))));
}
