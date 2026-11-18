use super::{quick_access_auto_scroll_delta, should_paint_item, vertical_drop_gap};
use crate::ui::sidebar::allocate_sidebar_viewport_row;
use eframe::egui::{pos2, vec2, Color32, Context, Event, Id, RawInput, Rect, Sense, Shape};
use std::cell::Cell;

#[test]
fn quick_access_auto_scrolls_only_near_vertical_edges_inside_width() {
    let viewport = Rect::from_min_max(pos2(10.0, 20.0), pos2(110.0, 220.0));

    assert!(quick_access_auto_scroll_delta(pos2(50.0, 25.0), viewport, 1.0 / 60.0) < 0.0);
    assert!(quick_access_auto_scroll_delta(pos2(50.0, 215.0), viewport, 1.0 / 60.0) > 0.0);
    assert_eq!(
        quick_access_auto_scroll_delta(pos2(50.0, 120.0), viewport, 1.0 / 60.0),
        0.0
    );
    assert_eq!(
        quick_access_auto_scroll_delta(pos2(5.0, 25.0), viewport, 1.0 / 60.0),
        0.0
    );

    let short_viewport = Rect::from_min_max(pos2(10.0, 20.0), pos2(110.0, 44.0));
    assert!(quick_access_auto_scroll_delta(pos2(50.0, 22.0), short_viewport, 1.0 / 60.0) < 0.0);
    assert!(quick_access_auto_scroll_delta(pos2(50.0, 42.0), short_viewport, 1.0 / 60.0) > 0.0);
}

#[test]
fn auto_scroll_speed_ramps_with_edge_proximity_and_is_frame_rate_independent() {
    let viewport = Rect::from_min_max(pos2(10.0, 20.0), pos2(110.0, 220.0));
    let barely_inside_edge = quick_access_auto_scroll_delta(pos2(50.0, 49.0), viewport, 1.0);
    let half_into_edge = quick_access_auto_scroll_delta(pos2(50.0, 35.0), viewport, 1.0);
    let beyond_edge = quick_access_auto_scroll_delta(pos2(50.0, 0.0), viewport, 0.05);

    assert!(barely_inside_edge < 0.0);
    assert!(half_into_edge < barely_inside_edge);
    assert_eq!(beyond_edge, -9.0);

    let at_60_hz: f32 = (0..60)
        .map(|_| quick_access_auto_scroll_delta(pos2(50.0, 35.0), viewport, 1.0 / 60.0))
        .sum();
    let at_144_hz: f32 = (0..144)
        .map(|_| quick_access_auto_scroll_delta(pos2(50.0, 35.0), viewport, 1.0 / 144.0))
        .sum();
    assert!((at_60_hz - at_144_hz).abs() < 0.01);
}

#[test]
fn vertical_drop_gap_uses_row_centers_and_ignores_outside_pointer() {
    let viewport = Rect::from_min_size(pos2(0.0, 0.0), vec2(160.0, 100.0));
    let rows = [
        Rect::from_min_size(pos2(0.0, 10.0), vec2(160.0, 28.0)),
        Rect::from_min_size(pos2(0.0, 40.0), vec2(160.0, 28.0)),
    ];

    assert_eq!(
        vertical_drop_gap(Some(pos2(50.0, 15.0)), &rows, viewport),
        Some(0)
    );
    assert_eq!(
        vertical_drop_gap(Some(pos2(50.0, 30.0)), &rows, viewport),
        Some(1)
    );
    assert_eq!(
        vertical_drop_gap(Some(pos2(50.0, 80.0)), &rows, viewport),
        Some(2)
    );
    assert_eq!(
        vertical_drop_gap(Some(pos2(170.0, 30.0)), &rows, viewport),
        None
    );
}

#[test]
fn offscreen_drag_source_keeps_its_visual_in_the_lifted_preview() {
    let ctx = Context::default();
    let viewport = Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 100.0));
    let offscreen_row = Rect::from_min_size(pos2(0.0, 120.0), vec2(100.0, 28.0));
    let mut shapes = Vec::new();

    let _ = ctx.run_ui(
        RawInput {
            screen_rect: Some(viewport),
            events: vec![Event::PointerMoved(pos2(20.0, 50.0))],
            ..RawInput::default()
        },
        |ui| {
            ui.set_clip_rect(viewport);
            assert!(!ui.is_rect_visible(offscreen_row));

            let mut preview = crate::ui::tab_drag_preview::TabDragPreview::default();
            let paint_start = crate::ui::tab_drag_preview::TabDragPreview::begin(ui);
            if should_paint_item(ui, offscreen_row, true) {
                ui.painter().rect_filled(offscreen_row, 0.0, Color32::RED);
            }
            preview.record(ui, offscreen_row, paint_start);
            preview.paint_at_vertical_gap(
                ui,
                Id::new("offscreen_drag_source_test"),
                Some(0),
                viewport,
                Some(0),
                Some(10.0),
            );

            shapes = ctx.graphics(|graphics| {
                graphics
                    .get(ui.layer_id())
                    .unwrap()
                    .all_entries()
                    .cloned()
                    .collect()
            });
        },
    );

    assert!(shapes.iter().any(|entry| {
        matches!(
            &entry.shape,
            Shape::Rect(rect) if rect.fill == Color32::RED && rect.rect.top() == 40.0
        )
    }));
}

#[test]
fn drive_rows_remain_at_viewport_width_after_wide_tree_content() {
    let ctx = Context::default();
    let tree_width = Cell::new(0.0);
    let drive_width = Cell::new(0.0);
    let sidebar_viewport_width = 220.0;

    let _ = ctx.run_ui(eframe::egui::RawInput::default(), |ui| {
        ui.set_width(sidebar_viewport_width);
        let (tree_rect, _) = ui.allocate_exact_size(vec2(640.0, 20.0), Sense::hover());
        let (drive_rect, _) =
            allocate_sidebar_viewport_row(ui, sidebar_viewport_width, 32.0, Sense::click());
        tree_width.set(tree_rect.width());
        drive_width.set(drive_rect.width());
    });

    assert_eq!(tree_width.get(), 640.0);
    assert_eq!(drive_width.get(), sidebar_viewport_width);
}
