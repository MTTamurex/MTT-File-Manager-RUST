use super::*;
use crate::ui::tab_drag_preview::TabDragPreview;

#[test]
fn auxiliary_window_cannot_take_focus_mouse_or_taskbar_space() {
    let builder = builder(egui::vec2(240.0, 72.0), egui::pos2(-500.0, 200.0));
    assert_eq!(builder.active, Some(false));
    assert_eq!(builder.mouse_passthrough, Some(true));
    assert_eq!(builder.taskbar, Some(false));
    assert_eq!(builder.decorations, Some(false));
    assert_eq!(builder.resizable, Some(false));
    assert_eq!(builder.transparent, Some(true));
    assert_eq!(builder.has_shadow, Some(false));
    assert_eq!(builder.window_level, Some(egui::WindowLevel::AlwaysOnTop));
}

#[test]
fn positioning_preserves_grip_with_negative_monitor_coordinates_and_mixed_dpi() {
    let screen = egui::pos2(-1200.0, 450.0);
    let grab = egui::vec2(40.0, 12.0);
    for scale in [1.0, 1.25, 1.5, 2.0, 3.0] {
        let origin = position(screen, scale, grab);
        let actual = (origin + grab + PADDING) * scale;
        assert!((actual - screen).length() < 0.01);
    }
}

fn frame(
    ctx: &egui::Context,
    pos: egui::Pos2,
    native_outside: bool,
    native_down: bool,
    source: Option<usize>,
    escape: bool,
) -> egui::FullOutput {
    let mut events = vec![egui::Event::PointerMoved(pos)];
    if escape {
        events.push(egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
    }
    ctx.run_ui(
        egui::RawInput {
            events,
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            ..Default::default()
        },
        |ui| {
            crate::ui::tab_bar::detach::set_test_pointer_at(
                ctx,
                native_outside,
                native_down,
                egui::pos2(-300.0, 120.0),
            );
            let mut preview = TabDragPreview::default();
            for index in 0..3 {
                let rect = egui::Rect::from_min_size(
                    egui::pos2(index as f32 * 200.0, 0.0),
                    egui::vec2(200.0, 36.0),
                );
                let start = TabDragPreview::begin(ui);
                ui.painter().rect_filled(rect, 8.0, egui::Color32::BLUE);
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Tab",
                    egui::FontId::proportional(13.0),
                    egui::Color32::WHITE,
                );
                preview.record(ui, rect, start);
            }
            preview.paint(
                ui,
                egui::Id::new("floating_test"),
                source,
                egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 36.0)),
            );
            // Simulate a later-rendered widget underneath the floating tab.
            ctx.set_cursor_icon(egui::CursorIcon::Text);
            super::finish_frame(ctx);
        },
    )
}

#[test]
fn floating_preview_replaces_the_source_and_returns_to_the_strip() {
    let ctx = egui::Context::default();
    ctx.set_embed_viewports(false);
    let inside = frame(&ctx, egui::pos2(70.0, 14.0), false, true, Some(0), false);
    assert!(!inside.viewport_output.contains_key(&viewport_id()));
    let content = frame(&ctx, egui::pos2(70.0, 100.0), false, true, Some(0), false);
    let preview = content.viewport_output.get(&viewport_id()).unwrap();
    assert_eq!(
        content.platform_output.cursor_icon,
        egui::CursorIcon::Default
    );
    assert_eq!(
        preview.builder.inner_size.unwrap(),
        preview_size(egui::vec2(200.0, 36.0))
    );
    assert!(matches!(content.shapes[0].shape, egui::Shape::Noop));
    let outside = frame(&ctx, egui::pos2(900.0, 100.0), true, true, Some(0), false);
    let preview = outside.viewport_output.get(&viewport_id()).unwrap();
    assert_eq!(
        preview.builder.inner_size.unwrap(),
        preview_size(egui::vec2(200.0, 36.0))
    );
    assert!(preview.viewport_ui_cb.is_some());
    let returned = frame(&ctx, egui::pos2(250.0, 14.0), false, true, Some(0), false);
    assert!(!returned.viewport_output.contains_key(&viewport_id()));
    assert_eq!(returned.platform_output.cursor_icon, egui::CursorIcon::Text);
    assert!(returned.shapes.iter().any(
        |shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == "Tab")
    ));
}

#[test]
fn native_position_supports_dragging_without_more_egui_pointer_events() {
    let ctx = egui::Context::default();
    ctx.set_embed_viewports(false);
    // The last egui pointer location is still over the tab, but Windows reports outside.
    let output = frame(&ctx, egui::pos2(70.0, 14.0), true, true, Some(0), false);
    let preview = output.viewport_output.get(&viewport_id()).unwrap();
    assert!(preview.builder.position.unwrap().x < 0.0);
    assert_eq!(
        preview.builder.inner_size.unwrap(),
        preview_size(egui::vec2(200.0, 36.0))
    );
}

#[test]
fn release_escape_or_lost_ownership_removes_the_auxiliary_viewport() {
    for (down, source, escape) in [
        (false, Some(0), false),
        (true, None, false),
        (true, Some(0), true),
    ] {
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        let active = frame(&ctx, egui::pos2(900.0, 100.0), true, true, Some(0), false);
        assert!(active.viewport_output.contains_key(&viewport_id()));
        let ended = frame(&ctx, egui::pos2(900.0, 100.0), true, down, source, escape);
        assert!(!ended.viewport_output.contains_key(&viewport_id()));
    }
}

#[test]
fn losing_focus_closes_an_existing_preview_even_while_the_button_is_down() {
    let ctx = egui::Context::default();
    ctx.set_embed_viewports(false);
    let active = frame(&ctx, egui::pos2(900.0, 100.0), true, true, Some(0), false);
    assert!(active.viewport_output.contains_key(&viewport_id()));
    let ended = ctx.run_ui(
        egui::RawInput {
            focused: false,
            ..Default::default()
        },
        |ui| {
            let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 36.0));
            assert!(!show(
                ui,
                (rect, ShapeIdx(0), ShapeIdx(0)),
                Some(NativePointer {
                    outside: true,
                    down: true,
                    screen_px: egui::pos2(900.0, 100.0),
                    owner: None,
                })
            ));
        },
    );
    assert!(!ended.viewport_output.contains_key(&viewport_id()));
}

#[test]
fn top_and_panel_previews_keep_the_tab_size_with_only_transparent_shadow_padding() {
    for height in [28.0, 36.0] {
        let tab = egui::vec2(100.0, height);
        assert_eq!(preview_size(tab), tab + PADDING * 2.0);
    }
}

#[test]
fn unsupported_backends_never_create_an_interactive_embedded_window() {
    let ctx = egui::Context::default();
    ctx.set_embed_viewports(true);
    let output = frame(&ctx, egui::pos2(900.0, 100.0), true, true, Some(0), false);
    assert!(!output.viewport_output.contains_key(&viewport_id()));
}

#[test]
fn rendering_contains_only_the_original_tab_and_its_contoured_shadow() {
    let ctx = egui::Context::default();
    let size = egui::vec2(200.0, 36.0);
    let tab = egui::Rect::from_min_size(PADDING.to_pos2(), size);
    let bounds = egui::Rect::from_min_size(egui::Pos2::ZERO, preview_size(size));
    let corners = egui::CornerRadius {
        nw: 8,
        ne: 8,
        sw: 0,
        se: 0,
    };
    let output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(bounds),
            ..Default::default()
        },
        |ui| {
            let title = ui.painter().layout_no_wrap(
                "Tab".into(),
                egui::FontId::proportional(13.0),
                egui::Color32::WHITE,
            );
            let shapes = vec![
                egui::Shape::rect_filled(tab, corners, egui::Color32::BLUE),
                egui::Shape::circle_filled(
                    tab.min + egui::vec2(16.0, 18.0),
                    6.0,
                    egui::Color32::RED,
                ),
                egui::Shape::galley(
                    tab.min + egui::vec2(30.0, 10.0),
                    title,
                    egui::Color32::WHITE,
                ),
            ];
            paint_tab(ui, size, &shapes);
        },
    );
    assert_eq!(
        output.shapes.len(),
        4,
        "shadow, tab, icon, title; no surrounding panel or message"
    );
    let egui::Shape::Rect(shadow) = &output.shapes[0].shape else {
        panic!("expected shadow");
    };
    assert!(shadow.blur_width > 0.0);
    assert!(shadow.fill.a() < u8::MAX);
    assert_eq!(shadow.corner_radius, corners + egui::CornerRadius::same(1));
    let egui::Shape::Rect(background) = &output.shapes[1].shape else {
        panic!("expected tab background");
    };
    assert_eq!(background.rect, tab);
    assert_eq!(background.corner_radius, corners);
    assert!(output.shapes.iter().all(|shape| !matches!(&shape.shape,
        egui::Shape::Rect(rect) if rect.rect == bounds)));
}
