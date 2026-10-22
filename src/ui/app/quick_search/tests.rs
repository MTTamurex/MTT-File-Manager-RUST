use super::*;
use crate::app::shortcuts::{ShortcutAction, ShortcutBinding, ShortcutBindings};

fn backspace() -> egui::Event {
    egui::Event::Key {
        key: egui::Key::Backspace,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

fn frame(events: Vec<egui::Event>, focused: bool) -> egui::RawInput {
    let mut raw = egui::RawInput {
        events,
        focused,
        ..Default::default()
    };
    raw.viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .focused = Some(focused);
    raw
}

#[test]
fn backspace_edits_unicode_and_consumes_the_last_character_before_navigation() {
    for query in ["file", "café", "🦀", "a"] {
        let ctx = egui::Context::default();
        let mut buffer = query.to_owned();
        let mut last = Instant::now();
        let now = last + Duration::from_millis(10);
        let mut shortcuts = ShortcutBindings::default();
        shortcuts.set(
            ShortcutAction::GoBack,
            ShortcutBinding::plain(egui::Key::Backspace),
        );
        let _ = ctx.run_ui(frame(vec![backspace()], true), |ui| {
            assert!(consume_backspace(ui.ctx(), &mut buffer, &mut last, now));
            assert!(!shortcuts.is_triggered(ShortcutAction::GoBack, ui.ctx()));
        });
        let mut expected = query.to_owned();
        expected.pop();
        assert_eq!(buffer, expected);
        assert_eq!(last, now);
    }
}

#[test]
fn empty_expired_and_unfocused_searches_leave_backspace_for_other_handlers() {
    for (query, age, focused) in [
        ("", Duration::ZERO, true),
        ("file", TIMEOUT + Duration::from_millis(1), true),
        ("file", Duration::ZERO, false),
    ] {
        let ctx = egui::Context::default();
        let mut buffer = query.to_owned();
        let mut last = Instant::now();
        let now = last + age;
        let _ = ctx.run_ui(frame(vec![backspace()], focused), |ui| {
            assert!(!consume_backspace(ui.ctx(), &mut buffer, &mut last, now));
            assert!(ui.input(|i| i.key_pressed(egui::Key::Backspace)));
        });
        if age > TIMEOUT {
            assert!(buffer.is_empty());
        }
    }
}

#[test]
fn focused_text_field_keeps_backspace_and_clears_quick_search() {
    let ctx = egui::Context::default();
    let mut buffer = "query".to_owned();
    let mut text = "file".to_owned();
    let id = egui::Id::new("quick_search_text_field");
    let _ = ctx.run_ui(frame(vec![], true), |ui| {
        ui.add(egui::TextEdit::singleline(&mut text).id(id))
            .request_focus();
    });
    let mut last = Instant::now();
    let now = last;
    let _ = ctx.run_ui(frame(vec![backspace()], true), |ui| {
        assert!(ui.ctx().egui_wants_keyboard_input());
        assert!(!consume_backspace(ui.ctx(), &mut buffer, &mut last, now));
        ui.add(egui::TextEdit::singleline(&mut text).id(id));
    });
    assert!(buffer.is_empty());
    assert_eq!(text, "fil");
}
