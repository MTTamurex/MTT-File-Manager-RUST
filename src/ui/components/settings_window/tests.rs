use super::*;
use crate::app::shortcuts::ShortcutEditorState;

fn frame(
    ctx: &egui::Context,
    section: &mut SettingsSection,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1024.0, 768.0),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            ui.set_width(SIDEBAR_WIDTH);
            render_settings_sidebar(ui, section);
        },
    )
}

fn click_label(ctx: &egui::Context, section: &mut SettingsSection, label: &str) {
    let output = frame(ctx, section, vec![]);
    let position = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::epaint::Shape::Text(text) if text.galley.text() == label => {
                Some(text.pos + text.galley.size() * 0.5)
            }
            _ => None,
        })
        .expect("settings section must be visible in the sidebar");
    for pressed in [true, false] {
        frame(
            ctx,
            section,
            vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
}

#[test]
fn appearance_has_its_own_settings_navigation_entry() {
    let ctx = egui::Context::default();
    let mut section = SettingsSection::General;
    click_label(&ctx, &mut section, &t!("settings.appearance"));
    assert_eq!(section, SettingsSection::Appearance);
    click_label(&ctx, &mut section, &t!("settings.general"));
    assert_eq!(section, SettingsSection::General);
}

#[test]
fn escape_cancels_capture_before_closing_settings_and_ignores_repeats() {
    let ctx = egui::Context::default();
    let mut editor = ShortcutEditorState::default();
    editor.begin_capture(crate::app::shortcuts::ShortcutAction::GoBack);
    editor.message = Some(crate::app::shortcuts::ShortcutValidationError::Reserved);

    for (pressed, close, capturing) in [
        (true, false, false),
        (true, false, false), // Holding Esc must not close the window.
        (false, false, false),
        (true, true, false), // A second independent press closes it.
    ] {
        let _ = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ui| {
                assert_eq!(settings_escape_closes_window(ui.ctx(), &mut editor), close);
                assert_eq!(editor.is_capturing(), capturing);
                assert!(editor.message.is_none());
                assert!(!ui.input(|input| input.key_pressed(egui::Key::Escape)));
            },
        );
    }
}

#[test]
fn escape_without_capture_closes_settings() {
    let ctx = egui::Context::default();
    let mut editor = ShortcutEditorState::default();
    let _ = ctx.run_ui(
        egui::RawInput {
            events: vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        },
        |ui| assert!(settings_escape_closes_window(ui.ctx(), &mut editor)),
    );
}

#[test]
fn escape_dismisses_a_settings_combo_popup_before_closing_the_window() {
    let ctx = egui::Context::default();
    let mut editor = ShortcutEditorState::default();
    let button = std::cell::Cell::new(egui::Pos2::ZERO);
    let mut draw = |ui: &mut egui::Ui, expect_close: bool| {
        assert_eq!(
            settings_escape_closes_window(ui.ctx(), &mut editor),
            expect_close
        );
        let combo = egui::ComboBox::from_id_salt("new_tab_location")
            .selected_text("Folder")
            .show_ui(ui, |ui| {
                ui.label("Folder");
            });
        button.set(combo.response.rect.center());
    };
    let _ = ctx.run_ui(egui::RawInput::default(), |ui| draw(ui, false));
    let point = button.get();
    for pressed in [true, false] {
        let _ = ctx.run_ui(
            egui::RawInput {
                events: vec![
                    egui::Event::PointerMoved(point),
                    egui::Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                ..Default::default()
            },
            |ui| draw(ui, false),
        );
    }
    assert!(egui::Popup::is_any_open(&ctx));
    // Give the newly opened popup its sizing pass.
    let _ = ctx.run_ui(egui::RawInput::default(), |ui| draw(ui, false));
    let escape = |pressed| egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    let _ = ctx.run_ui(
        egui::RawInput {
            events: vec![escape(true)],
            ..Default::default()
        },
        |ui| draw(ui, false),
    );
    assert!(!egui::Popup::is_any_open(&ctx));
    // Holding Escape after closing the popup must not close settings as well.
    let _ = ctx.run_ui(
        egui::RawInput {
            events: vec![escape(true)],
            ..Default::default()
        },
        |ui| draw(ui, false),
    );
    let _ = ctx.run_ui(
        egui::RawInput {
            events: vec![escape(false)],
            ..Default::default()
        },
        |ui| draw(ui, false),
    );
    let _ = ctx.run_ui(
        egui::RawInput {
            events: vec![escape(true)],
            ..Default::default()
        },
        |ui| draw(ui, true),
    );
}
