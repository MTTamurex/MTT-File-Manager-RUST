//! Hit testing for focus changes must respect popups above the file panes.

use eframe::egui;

pub(super) fn clicked_in_panel(ui: &egui::Ui, content: egui::Rect, tabs: egui::Rect) -> bool {
    ui.input(|i| i.pointer.primary_clicked())
        && (ui.rect_contains_pointer(content) || ui.rect_contains_pointer(tabs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::file_entry::SortMode;

    const MODES: [SortMode; 4] = [
        SortMode::Name,
        SortMode::Date,
        SortMode::Size,
        SortMode::Type,
    ];

    struct Fixture {
        ctx: egui::Context,
        mode: SortMode,
        combo: egui::Rect,
        options: Vec<(SortMode, egui::Rect)>,
        switch_focus: bool,
        panel_x: f32,
    }

    impl Fixture {
        fn new(panel_x: f32) -> Self {
            Self {
                ctx: Default::default(),
                mode: SortMode::Name,
                combo: egui::Rect::NOTHING,
                options: Vec::new(),
                switch_focus: false,
                panel_x,
            }
        }

        fn frame(&mut self, events: Vec<egui::Event>) {
            self.switch_focus = false;
            self.options.clear();
            let _ = self.ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(640.0, 400.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    ui.scope_builder(
                        egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
                            egui::pos2(self.panel_x + 10.0, 0.0),
                            egui::vec2(220.0, 30.0),
                        )),
                        |ui| {
                            self.combo = egui::ComboBox::from_id_salt("sort")
                                .selected_text(format!("{:?}", self.mode))
                                .show_ui(ui, |ui| {
                                    for mode in MODES {
                                        let response = ui.selectable_value(
                                            &mut self.mode,
                                            mode,
                                            format!("{mode:?}"),
                                        );
                                        self.options.push((mode, response.rect));
                                    }
                                })
                                .response
                                .rect;
                        },
                    );
                    let tabs = egui::Rect::from_min_size(
                        egui::pos2(self.panel_x, 30.0),
                        egui::vec2(300.0, 25.0),
                    );
                    let content = egui::Rect::from_min_size(
                        egui::pos2(self.panel_x, 55.0),
                        egui::vec2(300.0, 345.0),
                    );
                    // Like the real inactive pane, item widgets may own the click.
                    ui.interact(content, ui.id().with("file_pane"), egui::Sense::click());
                    self.switch_focus = clicked_in_panel(ui, content, tabs);
                },
            );
        }

        fn click(&mut self, position: egui::Pos2, button: egui::PointerButton) {
            self.frame(vec![egui::Event::PointerMoved(position)]);
            for pressed in [true, false] {
                self.frame(vec![egui::Event::PointerButton {
                    pos: position,
                    button,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }]);
            }
        }
    }

    #[test]
    fn choosing_sort_options_over_either_inactive_pane_keeps_focus() {
        for panel_x in [0.0, 320.0] {
            for mode in MODES {
                let mut fixture = Fixture::new(panel_x);
                fixture.frame(vec![]);
                fixture.click(fixture.combo.center(), egui::PointerButton::Primary);
                fixture.frame(vec![]);
                let position = fixture
                    .options
                    .iter()
                    .find(|(value, _)| *value == mode)
                    .expect("sort popup must be open")
                    .1
                    .center();
                assert!(position.y > 30.0, "popup must overlap the inactive pane");
                fixture.click(position, egui::PointerButton::Primary);
                assert_eq!(fixture.mode, mode);
                assert!(
                    !fixture.switch_focus,
                    "selecting {mode:?} must not activate the underlying pane"
                );
            }
        }
    }

    #[test]
    fn direct_primary_clicks_in_content_and_tabs_still_switch_focus() {
        for panel_x in [0.0, 320.0] {
            for y in [40.0, 100.0] {
                let mut fixture = Fixture::new(panel_x);
                fixture.frame(vec![]);
                fixture.click(egui::pos2(panel_x + 150.0, y), egui::PointerButton::Primary);
                assert!(fixture.switch_focus);
            }
        }
    }

    #[test]
    fn secondary_middle_and_outside_clicks_do_not_switch_focus() {
        for button in [egui::PointerButton::Secondary, egui::PointerButton::Middle] {
            let mut fixture = Fixture::new(0.0);
            fixture.frame(vec![]);
            fixture.click(egui::pos2(150.0, 100.0), button);
            assert!(!fixture.switch_focus);
        }
        let mut fixture = Fixture::new(0.0);
        fixture.frame(vec![]);
        fixture.click(egui::pos2(500.0, 100.0), egui::PointerButton::Primary);
        assert!(!fixture.switch_focus);
    }
}
