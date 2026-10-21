use super::*;

fn render_frame(
    ctx: &egui::Context,
    shortcuts: &mut ShortcutBindings,
    editor: &mut ShortcutEditorState,
) -> egui::FullOutput {
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 1400.0),
            )),
            ..Default::default()
        },
        |ui| {
            ui.set_width(900.0);
            assert!(!render_shortcut_settings_section(ui, shortcuts, editor));
        },
    )
}

fn reset_positions(output: &egui::FullOutput) -> Vec<f32> {
    let label = t!("settings.shortcuts_reset");
    output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::epaint::Shape::Text(text) if text.galley.text() == label => Some(text.pos.x),
            _ => None,
        })
        .collect()
}

#[test]
fn capture_hint_and_validation_errors_do_not_move_reset_buttons() {
    let ctx = egui::Context::default();
    let mut shortcuts = ShortcutBindings::default();
    let mut editor = ShortcutEditorState::default();
    // Let the grid remember its column widths before comparing frames.
    for _ in 0..3 {
        render_frame(&ctx, &mut shortcuts, &mut editor);
    }
    let baseline = reset_positions(&render_frame(&ctx, &mut shortcuts, &mut editor));
    assert_eq!(baseline.len(), ShortcutAction::CONFIGURABLE_COUNT);

    editor.begin_capture(ShortcutAction::GoBack);
    for error in [
        None,
        Some(ShortcutValidationError::Unsupported),
        Some(ShortcutValidationError::Conflict(
            ShortcutAction::PreviousViewMode,
        )),
    ] {
        editor.message = error;
        for _ in 0..3 {
            render_frame(&ctx, &mut shortcuts, &mut editor);
        }
        let output = render_frame(&ctx, &mut shortcuts, &mut editor);
        let positions = reset_positions(&output);
        assert_eq!(positions.len(), baseline.len());
        for (before, after) in baseline.iter().zip(positions) {
            assert!(
                (before - after).abs() < 1.0,
                "reset moved from {before} to {after}"
            );
        }
        assert!(output.shapes.iter().any(|shape| matches!(
            &shape.shape,
            egui::epaint::Shape::Text(text) if text.galley.text() == t!("settings.shortcuts_press_binding_hint")
        )));
    }
}
