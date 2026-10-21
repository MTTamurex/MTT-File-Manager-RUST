use super::section_menu;
use eframe::egui::{self, pos2, vec2, Context, Event, Modifiers, PointerButton, RawInput, Sense};

/// The panel-wide background target must not swallow a folder's context click.
#[test]
fn background_menu_yields_to_item_rows_but_opens_on_empty_space() {
    for (point, expect_background) in [(pos2(40.0, 20.0), false), (pos2(40.0, 120.0), true)] {
        let ctx = Context::default();
        let mut item_clicked = false;
        let mut background_open = false;
        let mut render = |ui: &mut egui::Ui| {
            let background = ui.interact(
                egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 200.0)),
                ui.id().with("background"),
                Sense::click(),
            );
            let row = ui.interact(
                egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 40.0)),
                ui.id().with("folder"),
                Sense::click(),
            );
            item_clicked |= row.secondary_clicked();
            section_menu::menu(&background, false);
            background_open = background.context_menu_opened();
        };
        let _ = ctx.run_ui(RawInput::default(), &mut render);
        for pressed in [true, false] {
            let _ = ctx.run_ui(
                RawInput {
                    events: vec![
                        Event::PointerMoved(point),
                        Event::PointerButton {
                            pos: point,
                            button: PointerButton::Secondary,
                            pressed,
                            modifiers: Modifiers::NONE,
                        },
                    ],
                    ..Default::default()
                },
                &mut render,
            );
        }
        assert_eq!(background_open, expect_background);
        assert_eq!(item_clicked, !expect_background);
    }
}

#[test]
fn section_header_menu_opens_without_empty_space() {
    let ctx = Context::default();
    let point = pos2(40.0, 20.0);
    let mut opened = false;
    let mut render = |ui: &mut egui::Ui| {
        let rect = egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 40.0));
        section_menu::header_menu(ui, rect, "section", false);
        let response = ui.interact(
            rect,
            ui.id().with(("section_menu", "section")),
            Sense::click(),
        );
        opened = response.context_menu_opened();
    };
    let _ = ctx.run_ui(RawInput::default(), &mut render);
    for pressed in [true, false] {
        let _ = ctx.run_ui(
            RawInput {
                events: vec![
                    Event::PointerMoved(point),
                    Event::PointerButton {
                        pos: point,
                        button: PointerButton::Secondary,
                        pressed,
                        modifiers: Modifiers::NONE,
                    },
                ],
                ..Default::default()
            },
            &mut render,
        );
    }
    assert!(opened);
}
