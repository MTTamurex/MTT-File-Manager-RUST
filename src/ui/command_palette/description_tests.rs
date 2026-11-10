use super::*;

fn render_at(size: egui::Vec2, scale: f32, command: Command) -> (egui::Context, egui::FullOutput) {
    let ctx = egui::Context::default();
    ctx.global_style_mut(|style| {
        style.animation_time = 0.0;
        for font in style.text_styles.values_mut() {
            font.size *= scale;
        }
    });
    let mut state = CommandPaletteState::default();
    state.open(None);
    state.selected = crate::app::command_palette::catalog::COMMANDS
        .iter()
        .position(|candidate| *candidate == command)
        .unwrap();
    let mut output = None;
    for _ in 0..12 {
        output = Some(ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                ..Default::default()
            },
            |ui| {
                assert!(render_dialog(
                    ui.ctx(),
                    &mut state,
                    &CommandContext::default(),
                    &ShortcutBindings::default()
                )
                .is_none());
            },
        ));
    }
    (ctx, output.unwrap())
}

#[test]
fn palette_title_has_a_blank_line_before_the_search_field() {
    for scale in [1.0, 1.5] {
        let (ctx, output) = render_at(egui::vec2(1200.0, 900.0), scale, Command::NewTab);
        let title = t!("command_palette.title");
        let title_bottom = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == title.as_ref() => {
                    Some(text.pos.y + text.galley.size().y)
                }
                _ => None,
            })
            .unwrap();
        let input = ctx.read_response(input_id()).unwrap();
        assert!(
            input.rect.top() - title_bottom
                >= egui::TextStyle::Body.resolve(&ctx.global_style()).size,
            "leave a font-scaled blank line between the palette title and search field"
        );
    }
}

#[test]
fn description_is_a_fixed_full_height_right_column_with_title_above_wrapped_text() {
    for size in [
        egui::vec2(900.0, 700.0),
        egui::vec2(1200.0, 900.0),
        egui::vec2(1600.0, 1000.0),
    ] {
        for scale in [1.0, 1.5] {
            let (ctx, output) = render_at(size, scale, Command::NewTab);
            let modal = ctx
                .memory(|memory| memory.area_rect(egui::Id::new("command_palette_modal")))
                .unwrap();
            assert!(
                egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(modal),
                "{modal:?}"
            );
            let help = input_hints::help(Command::NewTab, &ShortcutBindings::default());
            let title = t!("command_palette.description");
            let mut title_bounds = None;
            let mut help_bounds = None;
            let mut description_clip = None;
            let mut footer_right = None;
            for shape in &output.shapes {
                if let egui::Shape::Text(text) = &shape.shape {
                    let bounds = egui::Rect::from_min_size(text.pos, text.galley.size());
                    if text.galley.text() == title.as_ref() {
                        title_bounds = Some(bounds);
                        description_clip = Some(shape.clip_rect);
                    }
                    if text.galley.text() == help {
                        help_bounds = Some(bounds);
                    }
                    if text.galley.text() == t!("command_palette.hint").as_ref() {
                        footer_right = Some(bounds.right());
                    }
                    assert_ne!(
                        text.galley.text(),
                        t!("command_palette.tab_limit").as_ref(),
                        "disabled reason must not appear in the description"
                    );
                }
            }
            let title = title_bounds.unwrap();
            let help = help_bounds.unwrap();
            let clip = description_clip.unwrap();
            assert!(
                (clip.width() - layout::DESCRIPTION_WIDTH).abs() < 1.0,
                "{clip:?}"
            );
            assert!(
                (clip.height() - (modal.height() - 34.0)).abs() < 1.0,
                "{modal:?}, {clip:?}"
            );
            assert!((title.left() - help.left()).abs() < 1.0);
            assert!(title.bottom() <= help.top());
            assert!(
                help.top() - title.bottom()
                    >= egui::TextStyle::Body.resolve(&ctx.global_style()).size,
                "leave a blank line below the description title"
            );
            assert!(title.top() - clip.top() < 5.0);
            assert!(
                clip.expand(1.0).contains_rect(help),
                "description must fit without a scrollbar: {help:?}, {clip:?}, scale={scale}"
            );
            assert!(footer_right.unwrap() < clip.left());
        }
    }
}

#[test]
fn constrained_viewports_keep_the_two_columns_inside_the_window() {
    for size in [egui::vec2(700.0, 480.0), egui::vec2(450.0, 360.0)] {
        let (ctx, output) = render_at(size, 1.0, Command::Settings);
        let modal = ctx
            .memory(|memory| memory.area_rect(egui::Id::new("command_palette_modal")))
            .unwrap();
        assert!(
            egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(modal),
            "{modal:?}"
        );
        let title = t!("command_palette.description");
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == title.as_ref())));
    }
}

#[test]
fn description_title_is_translated_in_every_supported_language() {
    for locale in ["en", "pt-BR", "ru", "el", "zh-CN"] {
        assert_ne!(
            t!("command_palette.description", locale = locale).as_ref(),
            "command_palette.description"
        );
    }
}

#[test]
fn constrained_left_column_keeps_command_labels_shortcuts_and_footer_readable() {
    for size in [
        egui::vec2(700.0, 480.0),
        egui::vec2(900.0, 700.0),
        egui::vec2(1200.0, 900.0),
    ] {
        for scale in [1.0, 1.5] {
            let (_, output) = render_at(size, scale, Command::NewTab);
            let shortcuts = ShortcutBindings::default();
            for expected in [
                t!(Command::NewTab.label_key()).to_string(),
                input_hints::summary(Command::NewTab, &shortcuts),
                t!("command_palette.hint").to_string(),
            ] {
                let (bounds, clip) = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text)
                            if text.galley.text() == expected
                                || (text.galley.text().starts_with(&format!(
                                    "{}\n",
                                    t!(Command::NewTab.label_key())
                                )) && text.galley.text().contains(&expected)) =>
                        {
                            Some((
                                egui::Rect::from_min_size(text.pos, text.galley.size()),
                                shape.clip_rect,
                            ))
                        }
                        _ => None,
                    })
                    .expect("command label, shortcut and footer must all be painted");
                assert!(
                    clip.expand(1.0).contains_rect(bounds),
                    "{expected}: size={size:?}, scale={scale}, bounds={bounds:?}, clip={clip:?}"
                );
            }
        }
    }
}
