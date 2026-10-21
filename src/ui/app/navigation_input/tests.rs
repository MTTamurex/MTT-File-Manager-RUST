use super::*;
use crate::app::shortcuts::ShortcutTrigger;

fn key(key: egui::Key, modifiers: egui::Modifiers, repeat: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat,
        modifiers,
    }
}

fn read_frame(
    shortcuts: &ShortcutBindings,
    events: Vec<egui::Event>,
    text_input_active: bool,
    focused: bool,
) -> (NavigationInput, Vec<egui::Event>) {
    let ctx = egui::Context::default();
    let mut raw = egui::RawInput {
        events,
        focused,
        ..Default::default()
    };
    raw.viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .focused = Some(focused);
    let mut result = None;
    let _ = ctx.run_ui(raw, |ui| {
        let navigation = read_navigation_input(shortcuts, ui.ctx(), text_input_active);
        result = Some((navigation, ui.input(|input| input.events.clone())));
    });
    result.unwrap()
}

#[test]
fn default_navigation_keys_are_consumed() {
    for (key, expected) in [
        (egui::Key::ArrowLeft, (true, false, false)),
        (egui::Key::ArrowRight, (false, true, false)),
        (egui::Key::ArrowUp, (false, false, true)),
    ] {
        let (input, remaining) = read_frame(
            &ShortcutBindings::default(),
            vec![self::key(key, egui::Modifiers::ALT, false)],
            false,
            true,
        );
        assert_eq!((input.back, input.forward, input.up), expected);
        assert!(remaining.is_empty());
    }
}

#[test]
fn custom_backspace_replaces_default_and_is_consumed() {
    for action in [
        ShortcutAction::GoBack,
        ShortcutAction::GoForward,
        ShortcutAction::GoUp,
    ] {
        let mut shortcuts = ShortcutBindings::default();
        shortcuts.set(action, ShortcutBinding::plain(egui::Key::Backspace));
        let (input, remaining) = read_frame(
            &shortcuts,
            vec![key(egui::Key::Backspace, egui::Modifiers::NONE, false)],
            false,
            true,
        );
        assert_eq!(input.back, action == ShortcutAction::GoBack);
        assert_eq!(input.forward, action == ShortcutAction::GoForward);
        assert_eq!(input.up, action == ShortcutAction::GoUp);
        assert!(remaining.is_empty());

        let ShortcutTrigger::Key(old_key) = action.default_binding().trigger else {
            unreachable!()
        };
        let (input, remaining) = read_frame(
            &shortcuts,
            vec![key(old_key, egui::Modifiers::ALT, false)],
            false,
            true,
        );
        assert!(!input.back && !input.forward && !input.up);
        assert_eq!(remaining.len(), 1);
    }
}

#[test]
fn typing_and_unfocused_windows_do_not_consume_navigation_keys() {
    let mut shortcuts = ShortcutBindings::default();
    shortcuts.set(
        ShortcutAction::GoBack,
        ShortcutBinding::plain(egui::Key::Backspace),
    );
    for (typing, focused) in [(true, true), (false, false)] {
        let events = vec![key(egui::Key::Backspace, egui::Modifiers::NONE, false)];
        let (input, remaining) = read_frame(&shortcuts, events.clone(), typing, focused);
        assert!(!input.back && !input.forward && !input.up);
        assert_eq!(remaining, events);
    }
}

#[test]
fn repeats_are_consumed_without_navigating() {
    let ctx = egui::Context::default();
    let shortcuts = ShortcutBindings::default();
    let mut raw = egui::RawInput::default();
    raw.viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .focused = Some(true);
    // egui derives repeat from held-key state, not the incoming event flag.
    for expected_back in [true, false] {
        raw.events = vec![key(egui::Key::ArrowLeft, egui::Modifiers::ALT, false)];
        let _ = ctx.run_ui(raw.clone(), |ui| {
            let input = read_navigation_input(&shortcuts, ui.ctx(), false);
            assert_eq!(input.back, expected_back);
            assert!(!input.forward && !input.up);
            assert!(ui.input(|input| input.events.is_empty()));
        });
    }
}

#[test]
fn extra_modifiers_do_not_trigger_navigation() {
    let events = vec![key(
        egui::Key::ArrowLeft,
        egui::Modifiers::ALT | egui::Modifiers::SHIFT,
        false,
    )];
    let (input, remaining) = read_frame(&ShortcutBindings::default(), events.clone(), false, true);
    assert!(!input.back && !input.forward && !input.up);
    assert_eq!(remaining, events);
}

#[test]
fn backspace_edits_text_and_navigates_only_after_focus_is_released() {
    let ctx = egui::Context::default();
    let mut shortcuts = ShortcutBindings::default();
    shortcuts.set(
        ShortcutAction::GoBack,
        ShortcutBinding::plain(egui::Key::Backspace),
    );
    let field_id = egui::Id::new("navigation_test_text_field");
    let mut text = "example".to_owned();
    let mut raw = egui::RawInput::default();
    raw.viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .focused = Some(true);
    let _ = ctx.run_ui(raw.clone(), |ui| {
        let response = ui.add(egui::TextEdit::singleline(&mut text).id(field_id));
        response.request_focus();
    });

    raw.events = vec![key(egui::Key::Backspace, egui::Modifiers::NONE, false)];
    let _ = ctx.run_ui(raw.clone(), |ui| {
        assert!(ui.ctx().egui_wants_keyboard_input());
        let input =
            read_navigation_input(&shortcuts, ui.ctx(), ui.ctx().egui_wants_keyboard_input());
        assert!(!input.back);
        ui.add(egui::TextEdit::singleline(&mut text).id(field_id));
    });
    assert_eq!(text, "exampl");

    ctx.memory_mut(|memory| memory.surrender_focus(field_id));
    // Release the key before the next independent press.
    raw.events = vec![egui::Event::Key {
        key: egui::Key::Backspace,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }];
    let _ = ctx.run_ui(raw.clone(), |_| {});
    raw.events = vec![key(egui::Key::Backspace, egui::Modifiers::NONE, false)];
    let _ = ctx.run_ui(raw, |ui| {
        assert!(!ui.ctx().egui_wants_keyboard_input());
        let input =
            read_navigation_input(&shortcuts, ui.ctx(), ui.ctx().egui_wants_keyboard_input());
        assert!(input.back);
        assert!(ui.input(|input| input.events.is_empty()));
    });
}

#[test]
fn mouse_navigation_stays_fixed_even_with_custom_keys_and_text_focus() {
    let mut shortcuts = ShortcutBindings::default();
    shortcuts.set(
        ShortcutAction::GoBack,
        ShortcutBinding::plain(egui::Key::Backspace),
    );
    for (button, back, forward) in [
        (egui::PointerButton::Extra1, true, false),
        (egui::PointerButton::Extra2, false, true),
    ] {
        let events = vec![egui::Event::PointerButton {
            pos: egui::Pos2::ZERO,
            button,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        }];
        let (input, remaining) = read_frame(&shortcuts, events.clone(), true, true);
        assert_eq!(
            (input.back, input.forward, input.up),
            (back, forward, false)
        );
        assert_eq!(remaining, events);
    }
}
