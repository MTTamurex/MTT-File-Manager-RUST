use super::required_width;
use eframe::egui;

#[test]
fn automatic_and_compact_layouts_keep_viewport_width() {
    assert_eq!(required_width([300.0; 5], 400.0, true, false, false), 400.0);
}

#[test]
fn manual_columns_overflow_instead_of_shrinking() {
    let columns = [300.0, 170.0, 120.0, 100.0, 80.0];
    assert_eq!(required_width(columns, 400.0, false, false, false), 698.0);
    assert_eq!(required_width(columns, 900.0, false, false, false), 900.0);
}

#[test]
fn only_visible_columns_contribute_to_horizontal_overflow() {
    let columns = [300.0, 170.0, 120.0, 100.0, 80.0];
    assert_eq!(required_width(columns, 400.0, false, true, false), 578.0);
    assert_eq!(required_width(columns, 400.0, false, true, true), 578.0);
    assert_eq!(required_width(columns, 400.0, false, false, true), 778.0);
}

#[test]
fn horizontal_offset_is_clamped_after_resizing_or_reenabling_autofit() {
    let ctx = egui::Context::default();
    let mut scroll_x = 500.0;
    let _ = ctx.run_ui(Default::default(), |ui| {
        let rect = ui.available_rect_before_wrap();
        super::prepare_scrollbar(ui, rect, rect.width() + 200.0, &mut scroll_x, false, false);
        assert_eq!(scroll_x, 200.0);
        super::prepare_scrollbar(ui, rect, rect.width(), &mut scroll_x, false, false);
        assert_eq!(scroll_x, 0.0);
    });
}

#[test]
fn horizontal_wheel_scrolls_only_when_global_search_is_closed() {
    for global_search_active in [false, true] {
        let ctx = egui::Context::default();
        let mut scroll_x = 0.0;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(600.0, 400.0),
            )),
            events: vec![
                egui::Event::PointerMoved(egui::pos2(100.0, 100.0)),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(-20.0, 0.0),
                    phase: egui::TouchPhase::Move,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            ..Default::default()
        };
        let _ = ctx.run_ui(input, |ui| {
            let rect = ui.available_rect_before_wrap();
            super::prepare_scrollbar(
                ui,
                rect,
                rect.width() + 300.0,
                &mut scroll_x,
                global_search_active,
                false,
            );
        });
        assert_eq!(scroll_x, if global_search_active { 0.0 } else { 100.0 });
    }
}

fn scrollbar_frame(ctx: &egui::Context, scroll_x: &mut f32, events: Vec<egui::Event>) {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(600.0, 400.0),
        )),
        events,
        ..Default::default()
    };
    let _ = ctx.run_ui(input, |ui| {
        let rect = egui::Rect::from_min_size(egui::pos2(10.0, 10.0), egui::vec2(400.0, 200.0));
        super::prepare_scrollbar(ui, rect, 800.0, scroll_x, false, false);
    });
}

#[test]
fn dragging_thumb_from_its_edge_does_not_jump_to_pointer_center() {
    let ctx = egui::Context::default();
    let mut scroll_x = 100.0;
    // The thumb spans x=60..260. Grab near its left edge, not its center.
    let press = egui::pos2(80.0, 203.0);
    scrollbar_frame(&ctx, &mut scroll_x, vec![egui::Event::PointerMoved(press)]);
    scrollbar_frame(
        &ctx,
        &mut scroll_x,
        vec![egui::Event::PointerButton {
            pos: press,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    scrollbar_frame(
        &ctx,
        &mut scroll_x,
        vec![egui::Event::PointerMoved(egui::pos2(90.0, 203.0))],
    );
    assert_eq!(scroll_x, 120.0);
    scrollbar_frame(&ctx, &mut scroll_x, vec![]);
    assert_eq!(scroll_x, 120.0);
}

#[test]
fn clicking_scrollbar_track_moves_to_the_clicked_position() {
    let ctx = egui::Context::default();
    let mut scroll_x = 0.0;
    let click = egui::pos2(390.0, 203.0);
    scrollbar_frame(&ctx, &mut scroll_x, vec![egui::Event::PointerMoved(click)]);
    for pressed in [true, false] {
        scrollbar_frame(
            &ctx,
            &mut scroll_x,
            vec![egui::Event::PointerButton {
                pos: click,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            }],
        );
    }
    assert_eq!(scroll_x, 400.0);
}

#[test]
fn clicking_thumb_without_dragging_preserves_scroll_offset() {
    let ctx = egui::Context::default();
    let mut scroll_x = 100.0;
    let click = egui::pos2(80.0, 203.0);
    scrollbar_frame(&ctx, &mut scroll_x, vec![egui::Event::PointerMoved(click)]);
    for pressed in [true, false] {
        scrollbar_frame(
            &ctx,
            &mut scroll_x,
            vec![egui::Event::PointerButton {
                pos: click,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            }],
        );
    }
    assert_eq!(scroll_x, 100.0);
}
