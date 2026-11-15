use eframe::egui;
use rust_i18n::t;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DestinationConfirmationAction {
    Confirm,
    Cancel,
}

pub(super) fn render_destination_confirmation_dialog(
    ctx: &egui::Context,
    existing_path: &str,
    new_path: &str,
) -> egui::ModalResponse<Option<DestinationConfirmationAction>> {
    egui::Modal::new(egui::Id::new("organizer_rename_destination_confirmation"))
        .frame(egui::Frame::window(&ctx.global_style()).inner_margin(16.0))
        .show(ctx, |ui| {
            let mut action = None;
            ui.set_max_width((ctx.content_rect().width() - 48.0).max(1.0));
            ui.heading(t!("organizer.rename_destination_confirm_title"));
            ui.add_space(4.0);
            ui.label(t!("organizer.rename_destination_warning"));
            ui.add_space(6.0);
            ui.label(existing_path);
            ui.label("->");
            ui.label(new_path);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui
                    .button(t!("organizer.rename_destination_confirm"))
                    .clicked()
                {
                    action = Some(DestinationConfirmationAction::Confirm);
                }
                if ui.button(t!("organizer.cancel")).clicked() {
                    action = Some(DestinationConfirmationAction::Cancel);
                }
            });
            action
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_input(events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1024.0, 768.0),
            )),
            events,
            ..Default::default()
        }
    }

    fn render(ctx: &egui::Context) -> egui::ModalResponse<Option<DestinationConfirmationAction>> {
        render_destination_confirmation_dialog(
            ctx,
            "C:\\Library\\existing.txt",
            "C:\\Library\\renamed.txt",
        )
    }

    fn text_position(output: &egui::FullOutput, text: &str) -> egui::Pos2 {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text_shape) if text_shape.galley.text() == text => {
                    Some(text_shape.pos + text_shape.galley.size() * 0.5)
                }
                _ => None,
            })
            .expect("confirmation button label should be rendered")
    }

    #[test]
    fn destination_confirmation_is_centered_and_owns_the_top_modal_layer() {
        let ctx = egui::Context::default();
        // Modal ordering is committed at the end of a pass and becomes the
        // top-modal layer at the start of the following pass.
        let _ = ctx.run_ui(raw_input(vec![]), |_| {
            let _ = render(&ctx);
        });
        let output = ctx.run_ui(raw_input(vec![]), |ui| {
            let _settings_window = egui::Area::new(egui::Id::new("settings_window_test"))
                .order(egui::Order::Foreground)
                .fixed_pos(egui::pos2(24.0, 24.0))
                .show(ui.ctx(), |ui| {
                    let _ = ui.button("Settings control");
                });
            let dialog = render(&ctx);

            assert!(dialog.is_top_modal);
            assert_eq!(dialog.response.rect.center(), ctx.content_rect().center());
            assert!(ctx.memory(|memory| memory.top_modal_layer().is_some()));
        });
        let _ = text_position(
            &output,
            t!("organizer.rename_destination_confirm_title").as_ref(),
        );
    }

    #[test]
    fn clicking_the_backdrop_dismisses_confirmation() {
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(raw_input(vec![]), |ui| {
            egui::Area::new(egui::Id::new("settings_window_test"))
                .order(egui::Order::Foreground)
                .fixed_pos(egui::pos2(24.0, 24.0))
                .show(ui.ctx(), |ui| {
                    let _ = ui.button("Settings control");
                });
            let dialog = render(&ctx);
            assert!(!dialog.should_close());
        });

        let outside_dialog = egui::pos2(1010.0, 750.0);
        let _ = ctx.run_ui(
            raw_input(vec![egui::Event::PointerMoved(outside_dialog)]),
            |_| {
                let dialog = render(&ctx);
                assert!(!dialog.should_close());
            },
        );
        let _ = ctx.run_ui(
            raw_input(vec![egui::Event::PointerButton {
                pos: outside_dialog,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }]),
            |_| {
                let dialog = render(&ctx);
                assert!(!dialog.should_close());
            },
        );
        let _ = ctx.run_ui(
            raw_input(vec![egui::Event::PointerButton {
                pos: outside_dialog,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }]),
            |_| {
                let dialog = render(&ctx);
                assert!(dialog.should_close());
                assert!(dialog.backdrop_response.clicked());
            },
        );
    }

    #[test]
    fn escape_dismisses_confirmation_without_closing_settings_window() {
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(raw_input(vec![]), |ui| {
            let _ = render(&ctx);
            assert!(!ui.ctx().input(|input| input.key_pressed(egui::Key::Escape)));
        });
        let _ = ctx.run_ui(
            raw_input(vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }]),
            |_| {
                let dialog = render(&ctx);
                assert!(dialog.should_close());
            },
        );
    }

    #[test]
    fn confirmation_buttons_keep_their_actions() {
        for (label, expected_action) in [
            (
                t!("organizer.rename_destination_confirm").to_string(),
                DestinationConfirmationAction::Confirm,
            ),
            (
                t!("organizer.cancel").to_string(),
                DestinationConfirmationAction::Cancel,
            ),
        ] {
            let ctx = egui::Context::default();
            let _ = ctx.run_ui(raw_input(vec![]), |_| {
                let _ = render(&ctx);
            });
            let output = ctx.run_ui(raw_input(vec![]), |_| {
                let _ = render(&ctx);
            });
            let position = text_position(&output, &label);
            let action = std::cell::Cell::new(None);

            for pressed in [true, false] {
                let _ = ctx.run_ui(
                    raw_input(vec![
                        egui::Event::PointerMoved(position),
                        egui::Event::PointerButton {
                            pos: position,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ]),
                    |_| {
                        action.set(render(&ctx).inner);
                    },
                );
            }

            assert_eq!(action.get(), Some(expected_action));
        }
    }
}
