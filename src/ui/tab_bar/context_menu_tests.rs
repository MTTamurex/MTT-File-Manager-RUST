use super::context_menu::{close_control, tab_context_menu};
use crate::tabs::TabAction;
use eframe::egui::{self, pos2, vec2, Context, Event, Modifiers, PointerButton, RawInput, Sense};

fn pointer_input(pos: egui::Pos2, button: PointerButton, pressed: bool) -> RawInput {
    RawInput {
        events: vec![
            Event::PointerMoved(pos),
            Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers: Modifiers::NONE,
            },
        ],
        ..Default::default()
    }
}

#[test]
fn tab_context_menu_duplicates_the_clicked_tab_without_selecting_it_first() {
    let ctx = Context::default();
    let mut action = None;
    let mut selected = false;
    let mut render = |ui: &mut egui::Ui| {
        let response = ui.allocate_response(vec2(180.0, 36.0), Sense::click());
        selected |= response.clicked();
        if let Some(command) = tab_context_menu(&response, false, true, false, false) {
            action = Some(command);
        }
    };
    let _ = ctx.run_ui(RawInput::default(), &mut render);
    let point = pos2(40.0, 20.0);
    let _ = ctx.run_ui(
        pointer_input(point, PointerButton::Secondary, true),
        &mut render,
    );
    let _ = ctx.run_ui(
        pointer_input(point, PointerButton::Secondary, false),
        &mut render,
    );
    // Popups use an initial sizing pass before exposing their painted rows.
    let output = ctx.run_ui(RawInput::default(), &mut render);
    let duplicate = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::epaint::Shape::Text(text)
                if text.galley.text() == rust_i18n::t!("tabs.duplicate") =>
            {
                Some(shape.shape.visual_bounding_rect().center())
            }
            _ => None,
        })
        .expect("duplicate command is visible");
    for pressed in [true, false] {
        let _ = ctx.run_ui(
            pointer_input(duplicate, PointerButton::Primary, pressed),
            &mut render,
        );
    }
    assert_eq!(action, Some(TabAction::Duplicate));
    assert!(!selected);
}

#[test]
fn pinned_close_control_is_not_clickable() {
    for pinned in [false, true] {
        let ctx = Context::default();
        let mut clicked = false;
        let mut render = |ui: &mut egui::Ui| {
            let rect = egui::Rect::from_min_size(pos2(10.0, 10.0), vec2(16.0, 16.0));
            clicked |= close_control(
                ui,
                rect,
                ui.id().with("close"),
                pinned,
                egui::Color32::GRAY,
                egui::Color32::WHITE,
                egui::Color32::GRAY,
            )
            .clicked();
        };
        let _ = ctx.run_ui(RawInput::default(), &mut render);
        for pressed in [true, false] {
            let _ = ctx.run_ui(
                pointer_input(pos2(18.0, 18.0), PointerButton::Primary, pressed),
                &mut render,
            );
        }
        assert_eq!(clicked, !pinned);
    }
}
