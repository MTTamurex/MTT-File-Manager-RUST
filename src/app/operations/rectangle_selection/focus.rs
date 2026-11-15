use eframe::egui;

pub(in crate::app::operations) fn release_rectangle_selection_focus(ctx: &egui::Context) {
    // A background drag does not produce the click that normally blurs TextEdit.
    // Release focus only after the file view has accepted a rectangle selection.
    ctx.memory_mut(|memory| {
        if let Some(id) = memory.focused() {
            memory.surrender_focus(id);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::release_rectangle_selection_focus;
    use eframe::egui;

    fn delete_key() -> egui::Event {
        egui::Event::Key {
            key: egui::Key::Delete,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn rectangle_selection_releases_search_focus_and_preserves_filter() {
        let ctx = egui::Context::default();
        let field_id = egui::Id::new("local_search_focus_test");
        let mut query = String::from("file");

        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.add(egui::TextEdit::singleline(&mut query).id(field_id))
                .request_focus();
        });
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.add(egui::TextEdit::singleline(&mut query).id(field_id));
            assert!(ctx.egui_wants_keyboard_input());
            release_rectangle_selection_focus(&ctx);
            assert!(!ctx.egui_wants_keyboard_input());
            assert_eq!(query, "file");
        });
        let _ = ctx.run_ui(
            egui::RawInput {
                events: vec![delete_key()],
                ..Default::default()
            },
            |ui| {
                // Match the shortcut guard without invoking a filesystem operation.
                assert!(!ctx.egui_wants_keyboard_input());
                ui.add(egui::TextEdit::singleline(&mut query).id(field_id));
                assert!(!ctx.egui_wants_keyboard_input());
                assert!(ui.input(|input| input.key_pressed(egui::Key::Delete)));
                assert_eq!(query, "file");
            },
        );
    }

    #[test]
    fn rectangle_selection_without_widget_focus_is_a_noop() {
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(egui::RawInput::default(), |_| {
            release_rectangle_selection_focus(&ctx);
            assert!(!ctx.egui_wants_keyboard_input());
        });
    }

    #[test]
    fn search_keeps_keyboard_focus_when_no_rectangle_selection_starts() {
        let ctx = egui::Context::default();
        let field_id = egui::Id::new("local_search_edit_test");
        let mut query = String::from("file");
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.add(egui::TextEdit::singleline(&mut query).id(field_id))
                .request_focus();
        });
        let _ = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Text(String::from("x"))],
                ..Default::default()
            },
            |ui| {
                ui.add(egui::TextEdit::singleline(&mut query).id(field_id));
                assert!(ctx.egui_wants_keyboard_input());
                assert!(query.contains('x'));
            },
        );
    }

    #[test]
    fn dragging_inside_search_selects_text_without_releasing_focus() {
        let ctx = egui::Context::default();
        let field_id = egui::Id::new("local_search_text_drag_test");
        let mut query = String::from("example filter");
        let mut rect = egui::Rect::NOTHING;
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            let response = ui.add(egui::TextEdit::singleline(&mut query).id(field_id));
            rect = response.rect;
            response.request_focus();
        });
        let start = egui::pos2(rect.left() + 4.0, rect.center().y);
        let end = egui::pos2(rect.left() + 55.0, rect.center().y);
        for events in [
            vec![
                egui::Event::PointerMoved(start),
                egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            vec![egui::Event::PointerMoved(end)],
            vec![egui::Event::PointerButton {
                pos: end,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        ] {
            let _ = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    ui.add(egui::TextEdit::singleline(&mut query).id(field_id));
                    assert!(ctx.egui_wants_keyboard_input());
                    assert_eq!(query, "example filter");
                },
            );
        }
        let selection = egui::TextEdit::load_state(&ctx, field_id)
            .unwrap()
            .cursor
            .char_range()
            .unwrap();
        assert!(!selection.is_empty());
    }
}
