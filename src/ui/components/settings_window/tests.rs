use super::*;

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
