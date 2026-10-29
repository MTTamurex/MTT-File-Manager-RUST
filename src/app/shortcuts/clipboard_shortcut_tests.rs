use super::*;

fn frame(modifiers: egui::Modifiers, events: Vec<egui::Event>, test: impl FnMut(&egui::Context)) {
    let ctx = egui::Context::default();
    let mut test = test;
    let _ = ctx.run_ui(
        egui::RawInput {
            modifiers,
            events,
            ..Default::default()
        },
        |ui| test(ui.ctx()),
    );
}

#[test]
fn windows_shift_delete_triggers_permanent_delete_without_cutting_or_recycling() {
    let shortcuts = ShortcutBindings::default();
    frame(egui::Modifiers::SHIFT, vec![egui::Event::Cut], |ctx| {
        assert!(shortcuts.is_triggered(ShortcutAction::DeletePermanently, ctx));
        assert!(!shortcuts.is_triggered(ShortcutAction::Cut, ctx));
        assert!(!shortcuts.is_triggered(ShortcutAction::Copy, ctx));
        assert!(!shortcuts.is_triggered(ShortcutAction::Paste, ctx));
        assert!(!shortcuts.is_triggered(ShortcutAction::Delete, ctx));
        // Recognition must not consume text-edit events or mutate clipboard output.
        assert!(ctx.input(|input| matches!(input.events.as_slice(), [egui::Event::Cut])));
        assert!(ctx.output(|output| output.commands.is_empty()));
    });
}

#[test]
fn ctrl_x_keeps_cut_semantics_for_native_and_explicit_key_events() {
    let shortcuts = ShortcutBindings::default();
    for events in [
        vec![egui::Event::Cut],
        vec![egui::Event::Key {
            key: egui::Key::X,
            physical_key: Some(egui::Key::X),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::CTRL,
        }],
    ] {
        frame(egui::Modifiers::CTRL, events, |ctx| {
            assert!(shortcuts.is_triggered(ShortcutAction::Cut, ctx));
            assert!(!shortcuts.is_triggered(ShortcutAction::DeletePermanently, ctx));
        });
    }
}

#[test]
fn explicit_delete_keys_keep_recycle_and_permanent_delete_separate() {
    let shortcuts = ShortcutBindings::default();
    for modifiers in [egui::Modifiers::NONE, egui::Modifiers::SHIFT] {
        frame(
            modifiers,
            vec![egui::Event::Key {
                key: egui::Key::Delete,
                physical_key: Some(egui::Key::Delete),
                pressed: true,
                repeat: false,
                modifiers,
            }],
            |ctx| {
                assert_eq!(
                    shortcuts.is_triggered(ShortcutAction::Delete, ctx),
                    !modifiers.shift
                );
                assert_eq!(
                    shortcuts.is_triggered(ShortcutAction::DeletePermanently, ctx),
                    modifiers.shift
                );
                assert!(!shortcuts.is_triggered(ShortcutAction::Cut, ctx));
            },
        );
    }
}

#[test]
fn capture_and_consumption_use_the_same_distinct_shift_delete_identity() {
    let shortcuts = ShortcutBindings::default();
    frame(egui::Modifiers::SHIFT, vec![egui::Event::Cut], |ctx| {
        assert_eq!(
            capture_shortcut(ctx),
            Some(ShortcutCapture::Binding(ShortcutBinding::shift(
                egui::Key::Delete
            )))
        );
        assert!(!shortcuts.consume_triggered(ShortcutAction::Cut, ctx));
        assert!(shortcuts.is_triggered(ShortcutAction::DeletePermanently, ctx));
        assert!(shortcuts.consume_triggered(ShortcutAction::DeletePermanently, ctx));
        assert!(!shortcuts.consume_triggered(ShortcutAction::DeletePermanently, ctx));
    });
}

#[test]
fn customized_bindings_do_not_turn_shift_delete_into_ctrl_x() {
    let mut shortcuts = ShortcutBindings::default();
    shortcuts.set(
        ShortcutAction::DeletePermanently,
        ShortcutBinding::ctrl_shift(egui::Key::D),
    );
    shortcuts.set(ShortcutAction::Cut, ShortcutBinding::ctrl(egui::Key::K));
    shortcuts.set(ShortcutAction::NewTab, ShortcutBinding::ctrl(egui::Key::X));
    frame(egui::Modifiers::SHIFT, vec![egui::Event::Cut], |ctx| {
        assert!(!shortcuts.is_triggered(ShortcutAction::Cut, ctx));
        assert!(!shortcuts.is_triggered(ShortcutAction::NewTab, ctx));
        assert!(!shortcuts.is_triggered(ShortcutAction::DeletePermanently, ctx));
    });
    frame(egui::Modifiers::CTRL, vec![egui::Event::Cut], |ctx| {
        assert!(shortcuts.is_triggered(ShortcutAction::NewTab, ctx));
        assert!(!shortcuts.is_triggered(ShortcutAction::Cut, ctx));
    });
    // Explicitly assigning Shift+Delete remains supported and is no longer seen as Ctrl+X.
    shortcuts.set(
        ShortcutAction::DeletePermanently,
        ShortcutBinding::shift(egui::Key::Delete),
    );
    frame(egui::Modifiers::SHIFT, vec![egui::Event::Cut], |ctx| {
        assert!(shortcuts.is_triggered(ShortcutAction::DeletePermanently, ctx));
    });
}

#[test]
fn shift_delete_still_cuts_selected_text_without_running_file_shortcuts() {
    let ctx = egui::Context::default();
    let id = egui::Id::new("text_cut_test");
    let mut text = "example".to_owned();
    let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
        let output = egui::TextEdit::singleline(&mut text).id(id).show(ui);
        output.response.request_focus();
        let mut state = output.state;
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(7),
            )));
        state.store(&ctx, id);
    });
    let output = ctx.run_ui(
        egui::RawInput {
            modifiers: egui::Modifiers::SHIFT,
            events: vec![egui::Event::Cut],
            ..Default::default()
        },
        |ui| {
            assert!(ctx.egui_wants_keyboard_input());
            assert!(!ShortcutBindings::default().is_triggered(ShortcutAction::Cut, &ctx));
            ui.add(egui::TextEdit::singleline(&mut text).id(id));
        },
    );
    assert!(text.is_empty());
    assert!(output.platform_output.commands.iter().any(
        |command| matches!(command, egui::OutputCommand::CopyText(value) if value == "example")
    ));
}
