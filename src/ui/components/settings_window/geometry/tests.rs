use super::*;

fn frame(ctx: &egui::Context, events: Vec<egui::Event>, size: egui::Vec2) -> egui::Rect {
    frame_with_saved(ctx, events, size, None)
}

fn frame_with_saved(
    ctx: &egui::Context,
    events: Vec<egui::Event>,
    size: egui::Vec2,
    saved: Option<SettingsWindowGeometry>,
) -> egui::Rect {
    let mut rect = egui::Rect::NOTHING;
    let _ = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |ui| {
            rect = window(ui.ctx(), saved)
                .frame(egui::Frame::new().stroke(egui::Stroke::new(1.0, egui::Color32::GRAY)))
                .show(ui.ctx(), |ui| {
                    ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), 48.0),
                        egui::Sense::hover(),
                    );
                    block_content_drag(ui);
                    super::super::body_scroll::show(ui, |ui| {
                        ui.allocate_space(ui.available_size());
                    });
                })
                .unwrap()
                .response
                .rect;
        },
    );
    rect
}

fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

#[test]
fn saved_geometry_is_restored_in_a_new_session_and_on_reopen() {
    let db = crate::infrastructure::app_state_db::AppStateDb::new_in_memory().unwrap();
    let expected = egui::Rect::from_min_size(egui::pos2(140.0, 90.0), egui::vec2(780.0, 570.0));
    let geometry = SettingsWindowGeometry::from_rect(expected).unwrap();
    db.set_preferences_batch(&[geometry.preference().unwrap()])
        .unwrap();

    let ctx = egui::Context::default();
    let size = egui::vec2(1600.0, 1200.0);
    let mut saved = SettingsWindowGeometry::load(&db);
    let mut rect = egui::Rect::NOTHING;
    for _ in 0..4 {
        rect = frame_with_saved(&ctx, vec![], size, saved);
        saved = SettingsWindowGeometry::from_rect(rect);
    }
    assert!(rect.min.distance(expected.min) < 2.0, "{rect:?}");
    assert!(rect.max.distance(expected.max) < 2.0, "{rect:?}");

    for _ in 0..2 {
        let _ = ctx.run_ui(Default::default(), |_| {});
    }
    for _ in 0..4 {
        rect = frame_with_saved(&ctx, vec![], size, saved);
        saved = SettingsWindowGeometry::from_rect(rect);
    }
    assert!(rect.min.distance(expected.min) < 2.0, "{rect:?}");
    assert!(rect.max.distance(expected.max) < 2.0, "{rect:?}");
}

#[test]
fn saved_geometry_is_constrained_to_a_smaller_viewport() {
    for position in [egui::pos2(1200.0, 900.0), egui::pos2(-500.0, -300.0)] {
        let ctx = egui::Context::default();
        let size = egui::vec2(640.0, 360.0);
        let mut saved = SettingsWindowGeometry::from_rect(egui::Rect::from_min_size(
            position,
            egui::vec2(900.0, 700.0),
        ));
        let mut rect = egui::Rect::NOTHING;
        for _ in 0..4 {
            rect = frame_with_saved(&ctx, vec![], size, saved);
            saved = SettingsWindowGeometry::from_rect(rect);
        }
        assert!(rect.min.x >= 0.0 && rect.min.y >= 0.0, "{rect:?}");
        assert!(rect.max.x <= size.x && rect.max.y <= size.y, "{rect:?}");
    }
}

#[test]
fn resizing_edges_and_corners_tracks_the_pointer_and_keeps_opposite_edges_fixed() {
    let size = egui::vec2(1600.0, 1200.0);
    for (x, y) in [
        (-1, 0),
        (1, 0),
        (0, -1),
        (0, 1),
        (-1, -1),
        (1, -1),
        (-1, 1),
        (1, 1),
    ] {
        let ctx = egui::Context::default();
        let mut before = egui::Rect::NOTHING;
        for _ in 0..4 {
            before = frame(&ctx, vec![], size);
        }
        let start = egui::pos2(
            match x {
                -1 => before.left(),
                1 => before.right(),
                _ => before.center().x,
            },
            match y {
                -1 => before.top(),
                1 => before.bottom(),
                _ => before.center().y,
            },
        );
        frame(&ctx, vec![egui::Event::PointerMoved(start)], size);
        frame(&ctx, vec![button(start, true)], size);
        let delta = egui::vec2(x as f32 * 60.0, y as f32 * 40.0);
        let end = start + delta;
        frame(&ctx, vec![egui::Event::PointerMoved(end)], size);
        let after = frame(&ctx, vec![], size);
        let expected = egui::Rect::from_min_max(
            before.min
                + egui::vec2(
                    if x < 0 { delta.x } else { 0.0 },
                    if y < 0 { delta.y } else { 0.0 },
                ),
            before.max
                + egui::vec2(
                    if x > 0 { delta.x } else { 0.0 },
                    if y > 0 { delta.y } else { 0.0 },
                ),
        );
        assert!(
            after.min.distance(expected.min) < 2.0,
            "{x},{y}: {before:?} -> {after:?}, expected {expected:?}"
        );
        assert!(
            after.max.distance(expected.max) < 2.0,
            "{x},{y}: {before:?} -> {after:?}, expected {expected:?}"
        );
        frame(&ctx, vec![button(end, false)], size);
        let released = frame(&ctx, vec![], size);
        assert!(released.min.distance(after.min) < 2.0);
        assert!(released.max.distance(after.max) < 2.0);
    }
}

#[test]
fn settings_center_only_on_first_open_and_preserve_geometry_on_reopen() {
    let ctx = egui::Context::default();
    let size = egui::vec2(1600.0, 1200.0);
    let mut rect = egui::Rect::NOTHING;
    for _ in 0..4 {
        rect = frame(&ctx, vec![], size);
    }
    assert!(rect.center().distance((size * 0.5).to_pos2()) < 2.0);
    let start = rect.right_center();
    frame(&ctx, vec![egui::Event::PointerMoved(start)], size);
    frame(&ctx, vec![button(start, true)], size);
    let end = start + egui::vec2(80.0, 0.0);
    frame(&ctx, vec![egui::Event::PointerMoved(end)], size);
    frame(&ctx, vec![], size);
    frame(&ctx, vec![button(end, false)], size);
    rect = frame(&ctx, vec![], size);
    assert!(rect.center().distance((size * 0.5).to_pos2()) > 20.0);
    let before_close = rect;

    for _ in 0..2 {
        let _ = ctx.run_ui(Default::default(), |_| {});
    }
    for _ in 0..3 {
        rect = frame(&ctx, vec![], size);
    }
    assert!(rect.min.distance(before_close.min) < 2.0);
    assert!(rect.max.distance(before_close.max) < 2.0);
}

#[test]
fn settings_remain_within_a_small_viewport() {
    let ctx = egui::Context::default();
    let size = egui::vec2(640.0, 360.0);
    for _ in 0..4 {
        let rect = frame(&ctx, vec![], size);
        assert!(
            rect.width() <= size.x && rect.height() <= size.y,
            "{rect:?}"
        );
    }
}

#[test]
fn resizing_below_the_old_minimum_and_up_to_the_viewport_limit() {
    let size = egui::vec2(1600.0, 1200.0);
    for delta in [egui::vec2(-500.0, -500.0), egui::vec2(1000.0, 1000.0)] {
        let ctx = egui::Context::default();
        let mut rect = egui::Rect::NOTHING;
        for _ in 0..4 {
            rect = frame(&ctx, vec![], size);
        }
        let start = rect.right_bottom();
        frame(&ctx, vec![egui::Event::PointerMoved(start)], size);
        frame(&ctx, vec![button(start, true)], size);
        let end = start + delta;
        frame(&ctx, vec![egui::Event::PointerMoved(end)], size);
        rect = frame(&ctx, vec![], size);
        let maximum = available_size(&ctx) + egui::vec2(2.0, 2.0);
        if delta.x < 0.0 {
            assert!(rect.width() < 700.0 && rect.height() < 420.0, "{rect:?}");
        }
        assert!(rect.width() <= maximum.x && rect.height() <= maximum.y);
        frame(&ctx, vec![button(end, false)], size);
    }
}

#[test]
fn dragging_the_content_does_not_move_the_settings_window() {
    let ctx = egui::Context::default();
    let size = egui::vec2(1600.0, 1200.0);
    let mut before = egui::Rect::NOTHING;
    for _ in 0..4 {
        before = frame(&ctx, vec![], size);
    }
    let start = before.center();
    frame(&ctx, vec![egui::Event::PointerMoved(start)], size);
    frame(&ctx, vec![button(start, true)], size);
    frame(
        &ctx,
        vec![egui::Event::PointerMoved(start + egui::vec2(60.0, 40.0))],
        size,
    );
    let after = frame(&ctx, vec![], size);
    assert!(after.min.distance(before.min) < 2.0);
    assert!(after.max.distance(before.max) < 2.0);
}

#[test]
fn dragging_the_header_moves_the_window_without_resizing_or_jumping_on_release() {
    let ctx = egui::Context::default();
    let size = egui::vec2(1600.0, 1200.0);
    // Mirror the application: capture the rendered geometry after each frame.
    let mut saved = None;
    let mut frame = |ctx: &egui::Context, events, size| {
        let rect = frame_with_saved(ctx, events, size, saved);
        saved = SettingsWindowGeometry::from_rect(rect);
        rect
    };
    let mut before = egui::Rect::NOTHING;
    for _ in 0..4 {
        before = frame(&ctx, vec![], size);
    }
    let start = egui::pos2(before.center().x, before.top() + 24.0);
    frame(&ctx, vec![egui::Event::PointerMoved(start)], size);
    frame(&ctx, vec![button(start, true)], size);
    let delta = egui::vec2(80.0, 50.0);
    let end = start + delta;
    frame(&ctx, vec![egui::Event::PointerMoved(end)], size);
    let moved = frame(&ctx, vec![], size);
    assert!(
        moved.min.distance(before.min + delta) < 2.0,
        "{before:?} -> {moved:?}"
    );
    assert!((moved.size() - before.size()).length() < 2.0);
    frame(&ctx, vec![button(end, false)], size);
    let released = frame(&ctx, vec![], size);
    assert!(released.min.distance(moved.min) < 2.0);
    assert!(released.max.distance(moved.max) < 2.0);

    // Moving must not interfere with the resize handles.
    let resize_start = released.left_top();
    frame(&ctx, vec![egui::Event::PointerMoved(resize_start)], size);
    frame(&ctx, vec![button(resize_start, true)], size);
    let resize_delta = egui::vec2(-40.0, -30.0);
    let resize_end = resize_start + resize_delta;
    frame(&ctx, vec![egui::Event::PointerMoved(resize_end)], size);
    let resized = frame(&ctx, vec![], size);
    assert!(resized.min.distance(released.min + resize_delta) < 2.0);
    assert!(resized.max.distance(released.max) < 2.0);
    frame(&ctx, vec![button(resize_end, false)], size);

    for _ in 0..2 {
        let _ = ctx.run_ui(Default::default(), |_| {});
    }
    let mut reopened = egui::Rect::NOTHING;
    for _ in 0..3 {
        reopened = frame(&ctx, vec![], size);
    }
    assert!(reopened.min.distance(resized.min) < 2.0);
    assert!(reopened.max.distance(resized.max) < 2.0);
}

#[test]
fn content_drag_blocker_preserves_checkbox_clicks_and_slider_drags() {
    let ctx = egui::Context::default();
    let checkbox = std::cell::Cell::new(egui::Rect::NOTHING);
    let slider = std::cell::Cell::new(egui::Rect::NOTHING);
    let mut checked = false;
    let mut value = 50.0;
    let mut draw = |events| {
        let _ = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1600.0, 1200.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                window(ui.ctx(), None).show(ui.ctx(), |ui| {
                    block_content_drag(ui);
                    checkbox.set(ui.checkbox(&mut checked, "Example setting").rect);
                    slider.set(
                        ui.add(egui::Slider::new(&mut value, 0.0..=100.0).show_value(false))
                            .rect,
                    );
                });
            },
        );
    };
    for _ in 0..4 {
        draw(vec![]);
    }
    let point = checkbox.get().center();
    draw(vec![egui::Event::PointerMoved(point)]);
    draw(vec![button(point, true)]);
    draw(vec![button(point, false)]);

    let start = slider.get().center();
    let end = slider.get().right_center() - egui::vec2(8.0, 0.0);
    draw(vec![egui::Event::PointerMoved(start)]);
    draw(vec![button(start, true)]);
    draw(vec![egui::Event::PointerMoved(end)]);
    draw(vec![]);
    draw(vec![button(end, false)]);
    assert!(checked);
    assert!(
        value > 80.0,
        "slider must still receive drag input: {value}"
    );
}
