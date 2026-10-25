use super::draw_header_resizable;
use crate::domain::file_entry::SortMode;
use eframe::egui;

fn frame(
    ctx: &egui::Context,
    pointer: Option<egui::Pos2>,
    sort_mode: SortMode,
    descending: bool,
) -> egui::FullOutput {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(600.0, 400.0),
        )),
        events: vec![match pointer {
            Some(pos) => egui::Event::PointerMoved(pos),
            None => egui::Event::PointerGone,
        }],
        ..Default::default()
    };
    ctx.run_ui(input, |ui| {
        let mut width = 200.0;
        draw_header_resizable(
            ui,
            "Name",
            &mut width,
            SortMode::Name,
            100.0,
            0.0,
            sort_mode,
            descending,
            600.0,
        );
        assert_eq!(width, 200.0);
    })
}

fn highlighted(output: &egui::FullOutput, ctx: &egui::Context) -> bool {
    output.shapes.iter().any(|shape| {
        matches!(&shape.shape, egui::Shape::Rect(rect)
            if rect.rect.size() == egui::vec2(200.0, 22.0)
                && rect.fill == crate::ui::theme::header_active_bg(ctx))
    })
}

fn text_color(output: &egui::FullOutput, text: &str) -> Option<egui::Color32> {
    output.shapes.iter().find_map(|shape| match &shape.shape {
        egui::Shape::Text(shape) if shape.galley.job.text == text => {
            Some(shape.galley.job.sections[0].format.color)
        }
        _ => None,
    })
}

#[test]
fn sorted_header_has_only_triangle_without_hover() {
    for visuals in [egui::Visuals::light(), egui::Visuals::dark()] {
        let ctx = egui::Context::default();
        ctx.set_visuals(visuals);
        for descending in [false, true] {
            let output = frame(&ctx, None, SortMode::Name, descending);
            assert!(!highlighted(&output, &ctx));
            let normal = crate::ui::theme::secondary_text_color(&ctx);
            assert_eq!(text_color(&output, "Name"), Some(normal));
            assert_eq!(
                text_color(&output, if descending { "▼" } else { "▲" }),
                Some(normal)
            );
        }
    }
}

#[test]
fn sorted_header_loses_highlight_when_pointer_leaves_but_keeps_triangle() {
    let ctx = egui::Context::default();
    let pointer = Some(egui::pos2(50.0, 10.0));
    frame(&ctx, pointer, SortMode::Name, true);
    let hovered = frame(&ctx, pointer, SortMode::Name, true);
    assert!(highlighted(&hovered, &ctx));
    assert_eq!(
        text_color(&hovered, "Name"),
        Some(crate::ui::theme::text_color(&ctx))
    );
    let departed = frame(&ctx, Some(egui::pos2(300.0, 100.0)), SortMode::Name, true);
    assert!(!highlighted(&departed, &ctx));
    assert!(text_color(&departed, "▼").is_some());
}

#[test]
fn header_loses_highlight_when_pointer_leaves_the_window() {
    let ctx = egui::Context::default();
    let pointer = Some(egui::pos2(50.0, 10.0));
    frame(&ctx, pointer, SortMode::Name, false);
    let hovered = frame(&ctx, pointer, SortMode::Name, false);
    assert!(highlighted(&hovered, &ctx));
    let departed = frame(&ctx, None, SortMode::Name, false);
    assert!(!highlighted(&departed, &ctx));
    assert!(text_color(&departed, "▲").is_some());
}

#[test]
fn unsorted_header_highlights_on_hover_without_a_triangle() {
    let ctx = egui::Context::default();
    let pointer = Some(egui::pos2(50.0, 10.0));
    frame(&ctx, pointer, SortMode::Date, false);
    let output = frame(&ctx, pointer, SortMode::Date, false);
    assert!(highlighted(&output, &ctx));
    assert!(text_color(&output, "▲").is_none());
    assert!(text_color(&output, "▼").is_none());
}

fn interaction_frame(
    ctx: &egui::Context,
    events: Vec<egui::Event>,
    width: &mut f32,
    available: f32,
) -> (bool, egui::Rect) {
    let mut clicked = false;
    let mut rect = egui::Rect::NOTHING;
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(600.0, 400.0),
        )),
        events,
        ..Default::default()
    };
    let _ = ctx.run_ui(input, |ui| {
        rect = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(*width, 22.0));
        clicked = draw_header_resizable(
            ui,
            "Name",
            width,
            SortMode::Name,
            100.0,
            0.0,
            SortMode::Name,
            false,
            available,
        );
    });
    (clicked, rect)
}

fn pointer_button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

#[test]
fn already_sorted_header_still_accepts_sort_clicks_in_both_layout_modes() {
    for available in [600.0, f32::INFINITY] {
        let ctx = egui::Context::default();
        let mut width = 200.0;
        let (_, rect) = interaction_frame(&ctx, vec![], &mut width, available);
        let pos = rect.center();
        interaction_frame(
            &ctx,
            vec![egui::Event::PointerMoved(pos)],
            &mut width,
            available,
        );
        let (clicked, _) =
            interaction_frame(&ctx, vec![pointer_button(pos, true)], &mut width, available);
        assert!(!clicked);
        let (clicked, _) = interaction_frame(
            &ctx,
            vec![pointer_button(pos, false)],
            &mut width,
            available,
        );
        assert!(clicked);
        assert_eq!(width, 200.0);
        let departed = frame(&ctx, None, SortMode::Name, true);
        assert!(!highlighted(&departed, &ctx));
        assert!(text_color(&departed, "▼").is_some());
    }
}

#[test]
fn resizing_header_does_not_trigger_sorting_in_both_layout_modes() {
    for available in [600.0, f32::INFINITY] {
        let ctx = egui::Context::default();
        let mut width = 200.0;
        let (_, rect) = interaction_frame(&ctx, vec![], &mut width, available);
        let pos = egui::pos2(rect.right() - 1.0, rect.center().y);
        interaction_frame(
            &ctx,
            vec![egui::Event::PointerMoved(pos)],
            &mut width,
            available,
        );
        interaction_frame(&ctx, vec![pointer_button(pos, true)], &mut width, available);
        let moved = pos + egui::vec2(20.0, 0.0);
        let (clicked, _) = interaction_frame(
            &ctx,
            vec![egui::Event::PointerMoved(moved)],
            &mut width,
            available,
        );
        assert!(!clicked);
        assert_eq!(width, 220.0);
        let (clicked, _) = interaction_frame(
            &ctx,
            vec![pointer_button(moved, false)],
            &mut width,
            available,
        );
        assert!(!clicked);
        assert_eq!(width, 220.0);
    }
}

#[test]
fn non_primary_buttons_cannot_resize_columns() {
    for button in [
        egui::PointerButton::Secondary,
        egui::PointerButton::Middle,
        egui::PointerButton::Extra1,
    ] {
        let ctx = egui::Context::default();
        let mut width = 200.0;
        let (_, rect) = interaction_frame(&ctx, vec![], &mut width, f32::INFINITY);
        let start = egui::pos2(rect.right() - 1.0, rect.center().y);
        let end = start + egui::vec2(40.0, 0.0);
        interaction_frame(
            &ctx,
            vec![egui::Event::PointerMoved(start)],
            &mut width,
            f32::INFINITY,
        );
        interaction_frame(
            &ctx,
            vec![egui::Event::PointerButton {
                pos: start,
                button,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
            &mut width,
            f32::INFINITY,
        );
        let (clicked, _) = interaction_frame(
            &ctx,
            vec![egui::Event::PointerMoved(end)],
            &mut width,
            f32::INFINITY,
        );
        assert!(!clicked);
        assert_eq!(width, 200.0, "button {button:?} resized the column");
    }
}
