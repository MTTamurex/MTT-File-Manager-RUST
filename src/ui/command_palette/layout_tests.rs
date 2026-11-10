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
            let expected_help = input_hints::help(Command::NewTab, &shortcuts);
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
                // The left column retains 640 points; the description has its own 320-point column.
                let scrollbar_left = modal.left() + 17.0 + 640.0 - bar_width;
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

#[test]
fn every_description_keeps_the_help_area_and_dialog_size_stable() {
    use crate::app::command_palette::catalog::COMMANDS;
    for scale in [1.0, 1.5] {
        let ctx = egui::Context::default();
        ctx.global_style_mut(|style| {
            style.animation_time = 0.0;
            for font in style.text_styles.values_mut() {
                font.size *= scale;
            }
        });
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 900.0));
        let mut state = CommandPaletteState::default();
        state.open(None);
        let shortcuts = ShortcutBindings::default();
        let mut baseline: Option<egui::Rect> = None;
        // Let the initial modal measurement and centering settle before comparing commands.
        for _ in 0..12 {
            let _ = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(viewport),
                    ..Default::default()
                },
                |ui| {
                    let _ =
                        render_dialog(ui.ctx(), &mut state, &CommandContext::default(), &shortcuts);
                },
            );
        }
        for (selected, &command) in COMMANDS.iter().enumerate() {
            state.selected = selected;
            let mut painted = false;
            for _ in 0..4 {
                let output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(viewport),
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
                let expected_help = input_hints::help(command, &shortcuts);
                painted = output.shapes.iter().any(|shape| {
                    matches!(&shape.shape,
                    egui::Shape::Text(text) if text.galley.text() == expected_help)
                });
                for shape in &output.shapes {
                    if let egui::Shape::Text(text) = &shape.shape {
                        if text.galley.text() == expected_help {
                            let bounds = egui::Rect::from_min_size(text.pos, text.galley.size());
                            assert!(shape.clip_rect.expand(1.0).contains_rect(bounds), "{command:?}: description must fit without scrolling: {bounds:?}, clip={:?}", shape.clip_rect);
                        }
                    }
                }
            }
            assert!(painted, "{command:?} must have a rendered description");
            let rect = ctx
                .memory(|memory| memory.area_rect(egui::Id::new("command_palette_modal")))
                .unwrap();
            let initial = *baseline.get_or_insert(rect);
            assert!(
                (initial.height() - rect.height()).abs() < 1.0,
                "{command:?}: initial={initial:?}, current={rect:?}, scale={scale}"
            );
            assert!(
                (initial.center().y - rect.center().y).abs() < 1.0,
                "{command:?}: initial={initial:?}, current={rect:?}, scale={scale}"
            );
            assert!(viewport.contains_rect(rect));
        }
    }
}
