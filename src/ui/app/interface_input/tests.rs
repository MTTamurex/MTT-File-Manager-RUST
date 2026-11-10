use super::*;

fn event(binding: ShortcutBinding, repeat: bool) -> egui::Event {
    let crate::app::shortcuts::ShortcutTrigger::Key(key) = binding.trigger else {
        unreachable!()
    };
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat,
        modifiers: egui::Modifiers {
            ctrl: binding.ctrl,
            alt: binding.alt,
            shift: binding.shift,
            ..Default::default()
        },
    }
}

fn frame(
    shortcuts: &ShortcutBindings,
    events: Vec<egui::Event>,
    enabled: bool,
    focused: bool,
) -> ((bool, Option<ShortcutAction>), Vec<egui::Event>) {
    let ctx = egui::Context::default();
    frame_with_context(&ctx, shortcuts, events, enabled, focused)
}

fn frame_with_context(
    ctx: &egui::Context,
    shortcuts: &ShortcutBindings,
    events: Vec<egui::Event>,
    enabled: bool,
    focused: bool,
) -> ((bool, Option<ShortcutAction>), Vec<egui::Event>) {
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
        let input = read_interface_input(shortcuts, ui.ctx(), enabled);
        result = Some((input, ui.input(|input| input.events.clone())));
    });
    result.unwrap()
}

#[test]
fn interface_keys_are_consumed_once_and_repeats_do_not_toggle() {
    let shortcuts = ShortcutBindings::default();
    for action in ACTIONS {
        let ctx = egui::Context::default();
        let binding = shortcuts.get(action);
        let (input, remaining) = frame_with_context(
            &ctx,
            &shortcuts,
            vec![event(binding, false), event(binding, true)],
            true,
            true,
        );
        assert_eq!(input, (true, Some(action)));
        assert!(remaining.is_empty());
        let (input, remaining) =
            frame_with_context(&ctx, &shortcuts, vec![event(binding, true)], true, true);
        assert_eq!(input, (true, None));
        assert!(remaining.is_empty());
    }
}

#[test]
fn editing_modals_or_missing_focus_leave_keys_untouched() {
    let shortcuts = ShortcutBindings::default();
    for action in ACTIONS {
        for (enabled, focused) in [(false, true), (true, false), (false, false)] {
            let events = vec![event(shortcuts.get(action), false)];
            let (input, remaining) = frame(&shortcuts, events.clone(), enabled, focused);
            assert_eq!(input, (false, None));
            assert_eq!(remaining, events);
        }
    }
}

#[test]
fn remapping_replaces_default_without_consuming_space_or_unrelated_keys() {
    for action in ACTIONS {
        let mut shortcuts = ShortcutBindings::default();
        let custom = ShortcutBinding::ctrl_shift(egui::Key::K);
        shortcuts.set(action, custom);
        let events = vec![
            event(action.default_binding(), false),
            event(ShortcutBinding::plain(egui::Key::Space), false),
        ];
        let (input, remaining) = frame(&shortcuts, events.clone(), true, true);
        assert_eq!(input, (false, None));
        assert_eq!(remaining, events);
        let (input, remaining) = frame(&shortcuts, vec![event(custom, false)], true, true);
        assert_eq!(input, (true, Some(action)));
        assert!(remaining.is_empty());
    }
}

#[test]
fn key_release_does_not_run_an_action() {
    let shortcuts = ShortcutBindings::default();
    let mut release = event(shortcuts.get(ShortcutAction::TogglePreviewPanel), false);
    if let egui::Event::Key { pressed, .. } = &mut release {
        *pressed = false;
    }
    assert_eq!(
        frame(&shortcuts, vec![release], true, true).0,
        (false, None)
    );
}
