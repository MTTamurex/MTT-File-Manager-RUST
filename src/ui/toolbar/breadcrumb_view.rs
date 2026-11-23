use super::ToolbarAction;
use crate::domain::special_paths::COMPUTER_VIEW_ID;
use crate::ui::{preview_panel::utils::truncate_text_to_fit, theme};
use eframe::egui;

#[derive(Debug, PartialEq)]
enum Item {
    Segment(usize),
    Overflow(std::ops::Range<usize>),
}

fn fit_segments(widths: &[f32], available: f32, link: f32, overflow: f32) -> Vec<Item> {
    let count = widths.len();
    if count == 0 {
        return Vec::new();
    }
    let full = widths.iter().sum::<f32>() + link * count.saturating_sub(1) as f32;
    if full <= available || count == 1 {
        return (0..count).map(Item::Segment).collect();
    }

    // Keep the root and a contiguous tail when both fit around the overflow.
    let mut tail_width = widths[1..].iter().sum::<f32>();
    for start in 2..count {
        tail_width -= widths[start - 1];
        let width = widths[0] + overflow + tail_width + link * (count - start + 1) as f32;
        if width <= available {
            return std::iter::once(Item::Segment(0))
                .chain(std::iter::once(Item::Overflow(1..start)))
                .chain((start..count).map(Item::Segment))
                .collect();
        }
    }

    // Even an oversized current folder must stay visible. Its button is
    // truncated to the remaining space, while ancestors remain in the popup.
    if overflow + link < available {
        let mut tail_width = widths.iter().sum::<f32>();
        for start in 1..count {
            tail_width -= widths[start - 1];
            let width = overflow + tail_width + link * (count - start) as f32;
            if width <= available {
                return std::iter::once(Item::Overflow(0..start))
                    .chain((start..count).map(Item::Segment))
                    .collect();
            }
        }
        vec![Item::Overflow(0..count - 1), Item::Segment(count - 1)]
    } else {
        vec![Item::Segment(count - 1)]
    }
}

pub(super) fn render(ui: &mut egui::Ui, segments: &[(String, String)]) -> Option<ToolbarAction> {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let sep_font = egui::FontId::proportional(14.0);
    let measure = |text: &str, font: &egui::FontId| {
        ui.painter()
            .layout_no_wrap(text.into(), font.clone(), theme::text_color(ui.ctx()))
            .size()
            .x
    };
    let padding = ui.spacing().button_padding.x * 2.0;
    let widths: Vec<_> = segments
        .iter()
        .map(|(name, _)| measure(name, &font) + padding)
        .collect();
    let separator = measure("›", &sep_font);
    let link = separator + ui.spacing().item_spacing.x * 2.0;
    let overflow_width = measure("…", &font) + padding;
    let items = fit_segments(&widths, ui.available_width(), link, overflow_width);
    let mut action = None;
    for (position, item) in items.iter().enumerate() {
        if position > 0 {
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(separator, ui.available_height()),
                egui::Sense::hover(),
            );
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "›",
                sep_font.clone(),
                theme::secondary_text_color(ui.ctx()),
            );
        }
        match item {
            Item::Segment(index) => {
                let (name, target) = &segments[*index];
                let color = if *index + 1 == segments.len() {
                    theme::text_color(ui.ctx())
                } else {
                    theme::secondary_text_color(ui.ctx())
                };
                if button(ui, name, widths[*index], &font, color).clicked() {
                    action = Some(navigation_action(target));
                }
            }
            Item::Overflow(hidden) => {
                let response = button(
                    ui,
                    "…",
                    overflow_width,
                    &font,
                    theme::secondary_text_color(ui.ctx()),
                );
                egui::Popup::menu(&response)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
                    .show(|ui| {
                        ui.set_min_width(180.0);
                        for index in hidden.clone() {
                            let (name, target) = &segments[index];
                            if ui.button(name).clicked() {
                                action = Some(navigation_action(target));
                            }
                        }
                    });
            }
        }
    }
    action
}

fn navigation_action(target: &str) -> ToolbarAction {
    if target == COMPUTER_VIEW_ID {
        ToolbarAction::NavigateToComputer
    } else {
        ToolbarAction::Navigate(target.into())
    }
}

fn button(
    ui: &mut egui::Ui,
    name: &str,
    width: f32,
    font: &egui::FontId,
    color: egui::Color32,
) -> egui::Response {
    let width = width.min(ui.available_width()).max(0.0);
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(width, ui.available_height()),
        egui::Sense::click(),
    );
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), name));
    if response.hovered() || response.has_focus() {
        let visuals = ui.style().interact(&response);
        ui.painter().rect_filled(rect, 3.0, visuals.weak_bg_fill);
        if response.has_focus() {
            ui.painter()
                .rect_stroke(rect, 3.0, visuals.bg_stroke, egui::StrokeKind::Inside);
        }
    }
    let text_rect = rect.shrink2(egui::vec2(ui.spacing().button_padding.x, 0.0));
    let text = truncate_text_to_fit(name, text_rect.width().max(0.0), font, ui);
    ui.painter()
        .with_clip_rect(rect.intersect(ui.clip_rect()))
        .text(
            egui::pos2(text_rect.left(), rect.center().y),
            egui::Align2::LEFT_CENTER,
            &text,
            font.clone(),
            color,
        );
    if text != name {
        response.on_hover_text(name)
    } else {
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(
        ctx: &egui::Context,
        width: f32,
        segments: &[(String, String)],
        events: Vec<egui::Event>,
    ) -> (Option<ToolbarAction>, Vec<egui::epaint::ClippedShape>) {
        let mut action = None;
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 500.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                let bounds =
                    egui::Rect::from_min_size(egui::pos2(10.0, 10.0), egui::vec2(width, 24.0));
                let mut trail = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(bounds)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                trail.spacing_mut().item_spacing.x = 2.0;
                trail.set_clip_rect(bounds);
                action = render(&mut trail, segments);
                assert!(trail.min_rect().right() <= bounds.right() + 0.01);
                assert!(trail.min_rect().bottom() <= bounds.bottom() + 0.01);
            },
        );
        (action, output.shapes)
    }

    #[test]
    fn exact_fit_includes_spacing_on_both_sides_of_separators() {
        assert_eq!(
            fit_segments(&[40.0, 60.0, 50.0], 170.0, 10.0, 20.0),
            vec![Item::Segment(0), Item::Segment(1), Item::Segment(2)]
        );
        assert_eq!(
            fit_segments(&[40.0, 60.0, 50.0], 169.0, 10.0, 20.0),
            vec![Item::Segment(0), Item::Overflow(1..2), Item::Segment(2)]
        );
    }

    #[test]
    fn narrow_layout_never_loses_or_duplicates_current_folder() {
        for count in 1..8 {
            for available in [1.0, 25.0, 60.0, 120.0, 300.0] {
                let items = fit_segments(&vec![100.0; count], available, 10.0, 20.0);
                assert_eq!(items.last(), Some(&Item::Segment(count - 1)));
                let mut covered = Vec::new();
                for item in items {
                    match item {
                        Item::Segment(index) => covered.push(index),
                        Item::Overflow(hidden) => covered.extend(hidden),
                    }
                }
                let expected = if available > 30.0 || count == 1 {
                    (0..count).collect::<Vec<_>>()
                } else {
                    vec![count - 1]
                };
                assert_eq!(covered, expected);
            }
        }
    }

    #[test]
    fn computer_root_uses_the_computer_navigation_action() {
        assert!(matches!(
            navigation_action(COMPUTER_VIEW_ID),
            ToolbarAction::NavigateToComputer
        ));
        assert!(
            matches!(navigation_action(r"D:\"), ToolbarAction::Navigate(path) if path == r"D:\")
        );
    }

    #[test]
    fn rendered_trails_fit_and_current_folder_remains_visible() {
        let segments = vec![
            ("This PC".into(), COMPUTER_VIEW_ID.into()),
            ("Archive (D:)".into(), r"D:\".into()),
            ("A long folder name 日本語".repeat(4), r"D:\Folder".into()),
        ];
        for width in [40.0, 100.0, 200.0, 500.0] {
            let ctx = egui::Context::default();
            let (_, shapes) = frame(&ctx, width, &segments, Vec::new());
            assert!(shapes.iter().any(|shape| matches!(
                &shape.shape,
                egui::Shape::Text(text) if text.galley.job.text != "›"
                    && text.galley.job.text != "…"
                    && !text.galley.job.text.is_empty()
            )));
        }
    }

    #[test]
    fn rendered_colors_follow_the_theme_and_segment_position() {
        let segments = vec![
            ("Provider".into(), r"C:\Cloud".into()),
            ("Account".into(), r"C:\Cloud".into()),
            ("Folder".into(), r"C:\Cloud\Folder".into()),
        ];
        for visuals in [egui::Visuals::light(), egui::Visuals::dark()] {
            let ctx = egui::Context::default();
            ctx.set_visuals(visuals);
            let (_, shapes) = frame(&ctx, 500.0, &segments, Vec::new());
            for (index, (name, _)) in segments.iter().enumerate() {
                let text = shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.job.text == *name => Some(text),
                        _ => None,
                    })
                    .expect("each breadcrumb should be painted");
                assert_eq!(
                    text.galley.job.sections[0].format.color,
                    if index + 1 == segments.len() {
                        theme::text_color(&ctx)
                    } else {
                        theme::secondary_text_color(&ctx)
                    },
                );
            }
        }
    }

    #[test]
    fn clicking_a_truncated_current_folder_uses_the_full_target() {
        let ctx = egui::Context::default();
        let segments = vec![("Folder".repeat(30), r"D:\Folder".into())];
        frame(&ctx, 100.0, &segments, Vec::new());
        let point = egui::pos2(60.0, 22.0);
        frame(
            &ctx,
            100.0,
            &segments,
            vec![egui::Event::PointerMoved(point)],
        );
        let mut action = None;
        for pressed in [true, false] {
            action = frame(
                &ctx,
                100.0,
                &segments,
                vec![egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
            )
            .0;
        }
        assert!(matches!(action, Some(ToolbarAction::Navigate(path)) if path == r"D:\Folder"));
    }

    #[test]
    fn overflow_popup_keeps_hidden_ancestors_clickable() {
        let ctx = egui::Context::default();
        let segments = vec![
            ("Computer".into(), COMPUTER_VIEW_ID.into()),
            ("Archive (D:)".into(), r"D:\".into()),
            ("Folder".repeat(20), r"D:\Folder".into()),
        ];
        frame(&ctx, 140.0, &segments, Vec::new());
        let point = egui::pos2(20.0, 22.0);
        frame(
            &ctx,
            140.0,
            &segments,
            vec![egui::Event::PointerMoved(point)],
        );
        for pressed in [true, false] {
            frame(
                &ctx,
                140.0,
                &segments,
                vec![egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        let (_, shapes) = frame(&ctx, 140.0, &segments, Vec::new());
        let computer = shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == "Computer" => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                _ => None,
            })
            .expect("overflow should list the hidden computer root");
        frame(
            &ctx,
            140.0,
            &segments,
            vec![egui::Event::PointerMoved(computer)],
        );
        let mut action = None;
        for pressed in [true, false] {
            action = frame(
                &ctx,
                140.0,
                &segments,
                vec![egui::Event::PointerButton {
                    pos: computer,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
            )
            .0;
        }
        assert!(matches!(action, Some(ToolbarAction::NavigateToComputer)));
    }
}
