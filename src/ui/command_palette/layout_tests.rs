use super::*;

#[test]
fn shortcut_text_stays_clear_of_the_expanded_scrollbar_and_help_uses_body_font() {
    for scale in [1.0, 1.5] {
        for bar_width in [10.0, 18.0] {
            let ctx = egui::Context::default();
            ctx.global_style_mut(|style| {
                style.animation_time = 0.0;
                style.spacing.scroll.floating = true;
                style.spacing.scroll.bar_width = bar_width;
                for font in style.text_styles.values_mut() {
                    font.size *= scale;
                }
            });
            let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 900.0));
            let mut state = CommandPaletteState::default();
            state.open(None);
            let shortcuts = ShortcutBindings::default();
            let expected_input = input_hints::summary(Command::NewTab, &shortcuts);
            let expected_help = input_hints::help(Command::NewTab, &shortcuts).unwrap();
            let expected_footer = t!("command_palette.hint").to_string();
            let mut pointer = egui::Pos2::ZERO;
            for frame in 0..8 {
                let output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(viewport),
                        time: Some(frame as f64),
                        events: vec![egui::Event::PointerMoved(pointer)],
                        ..Default::default()
                    },
                    |ui| {
                        let _ = render_dialog(
                            ui.ctx(),
                            &mut state,
                            &CommandContext::default(),
                            &shortcuts,
                        );
                    },
                );
                let modal = ctx
                    .memory(|memory| memory.area_rect(egui::Id::new("command_palette_modal")))
                    .unwrap();
                // The modal frame uses 16-point inner margins plus its border.
                let scrollbar_left = modal.right() - 17.0 - bar_width;
                pointer = egui::pos2(scrollbar_left + bar_width / 2.0, modal.center().y);
                if frame == 7 {
                    let mut found_inputs = false;
                    let mut found_help = false;
                    let mut found_footer = false;
                    let body_size = egui::TextStyle::Body.resolve(&ctx.global_style()).size;
                    for shape in &output.shapes {
                        if let egui::Shape::Text(text) = &shape.shape {
                            if text.galley.text() == expected_input {
                                found_inputs = true;
                                let right = text.pos.x + text.galley.size().x;
                                assert!(
                                    scrollbar_left - right >= 7.0,
                                    "shortcut overlaps scrollbar: {right}, {scrollbar_left}"
                                );
                                assert!(
                                    text.galley.job.sections[0].format.font_id.size
                                        >= body_size * 0.89
                                );
                            }
                            if text.galley.text() == expected_help {
                                found_help = true;
                                assert_eq!(
                                    text.galley.job.sections[0].format.font_id.size,
                                    body_size
                                );
                            }
                            if text.galley.text() == expected_footer {
                                found_footer = true;
                                assert_eq!(
                                    text.galley.job.sections[0].format.font_id.size,
                                    body_size
                                );
                            }
                        }
                    }
                    assert!(
                        found_inputs && found_help && found_footer,
                        "must paint shortcuts, readable help and readable footer"
                    );
                    assert!(
                        viewport.contains_rect(modal),
                        "palette must fit after enlarging help: {modal:?}"
                    );
                }
            }
        }
    }
}
