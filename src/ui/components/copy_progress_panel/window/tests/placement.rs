use super::*;

fn rect(ctx: &egui::Context) -> egui::Rect {
    ctx.memory(|memory| memory.area_rect(egui::Id::new(WINDOW_ID)))
        .unwrap()
}

fn settle(
    ctx: &egui::Context,
    runtime: &CopyRuntime,
    panel: &mut CopyPanelState,
    size: egui::Vec2,
) -> Vec<Text> {
    for _ in 0..8 {
        draw(ctx, runtime, panel, size);
    }
    draw(ctx, runtime, panel, size)
}

fn drag(
    ctx: &egui::Context,
    runtime: &CopyRuntime,
    panel: &mut CopyPanelState,
    size: egui::Vec2,
    start: egui::Pos2,
) {
    let end = start + egui::vec2(-100.0, 40.0);
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    for events in [
        vec![egui::Event::PointerMoved(start)],
        vec![button(start, true)],
        vec![egui::Event::PointerMoved(end)],
        vec![],
        vec![button(end, false)],
    ] {
        let _ = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events,
                ..Default::default()
            },
            |ui| {
                show(ui.ctx(), &runtime.jobs(), panel);
            },
        );
    }
}

#[test]
fn new_deletion_is_centered_even_after_cached_copy_geometry_and_remains_draggable() {
    for scale in [1.0, 2.0] {
        let ctx = egui::Context::default();
        crate::ui::interface_scale::apply(&ctx, scale);
        let size = egui::vec2(1200.0, 1000.0);
        let center = egui::pos2(size.x / 2.0, size.y / 2.0);
        let runtime = CopyRuntime::default();
        let copy = runtime.start(
            vec![PathBuf::from("source/file.txt")],
            PathBuf::from("target"),
        );
        let mut panel = CopyPanelState::default();
        let texts = settle(&ctx, &runtime, &mut panel, size);
        assert!(
            (rect(&ctx).center() - center).length() <= 1.0,
            "Initial copy at scale {scale}: {:?}",
            rect(&ctx)
        );
        let title = super::super::super::summary::Summary::from_jobs(&runtime.jobs(), &panel)
            .unwrap()
            .label();
        let title = texts.iter().find(|text| text.value == title).unwrap();
        drag(&ctx, &runtime, &mut panel, size, title.rect.center());
        settle(&ctx, &runtime, &mut panel, size);
        assert!((rect(&ctx).center() - center).length() > 20.0);
        copy.finish(crate::infrastructure::copy_engine::Outcome {
            kind: crate::infrastructure::copy_engine::OutcomeKind::Completed,
            message: None,
        });
        runtime.dismiss(runtime.jobs()[0].0);
        let _deletion = runtime.start_delete(vec![PathBuf::from("source/file.txt")], false);
        let texts = settle(&ctx, &runtime, &mut panel, size);
        assert!(
            (rect(&ctx).center() - center).length() <= 1.0,
            "Deletion must open centered at scale {scale}: {:?}",
            rect(&ctx)
        );
        let title = texts
            .iter()
            .find(|text| text.value == t!("delete_engine.header", count = 0))
            .unwrap();
        drag(&ctx, &runtime, &mut panel, size, title.rect.center());
        settle(&ctx, &runtime, &mut panel, size);
        let moved = rect(&ctx);
        assert!(
            (moved.center() - center).length() > 20.0,
            "Deletion window must remain draggable"
        );
        settle(&ctx, &runtime, &mut panel, size);
        assert!((rect(&ctx).center() - moved.center()).length() <= 1.0);
        let _next = runtime.start_delete(vec![PathBuf::from("source/other.txt")], false);
        settle(&ctx, &runtime, &mut panel, size);
        assert!(
            (rect(&ctx).center() - center).length() <= 1.0,
            "A new deletion must open centered"
        );
    }
}

#[test]
fn each_new_copy_centers_once_and_preserves_manual_position_between_updates() {
    for scale in [1.0, 2.0] {
        let ctx = egui::Context::default();
        crate::ui::interface_scale::apply(&ctx, scale);
        let size = egui::vec2(1200.0, 1000.0);
        let center = egui::pos2(size.x / 2.0, size.y / 2.0);
        // A preexisting window position must not override initial centering.
        let _ = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                ..Default::default()
            },
            |ui| {
                egui::Window::new("Geometry fixture")
                    .id(egui::Id::new(WINDOW_ID))
                    .default_pos(egui::pos2(20.0, 70.0))
                    .show(ui.ctx(), |ui| {
                        ui.label("Fixture");
                    });
            },
        );
        let runtime = CopyRuntime::default();
        let _copy = runtime.start(
            vec![PathBuf::from("source/file.txt")],
            PathBuf::from("target"),
        );
        let mut panel = CopyPanelState::default();
        let texts = settle(&ctx, &runtime, &mut panel, size);
        assert!(
            (rect(&ctx).center() - center).length() <= 1.0,
            "Copy must open centered at scale {scale}"
        );
        let title = super::super::super::summary::Summary::from_jobs(&runtime.jobs(), &panel)
            .unwrap()
            .label();
        let title = texts.iter().find(|text| text.value == title).unwrap();
        drag(&ctx, &runtime, &mut panel, size, title.rect.center());
        settle(&ctx, &runtime, &mut panel, size);
        let moved = rect(&ctx);
        assert!((moved.center() - center).length() > 20.0);
        settle(&ctx, &runtime, &mut panel, size);
        assert!((rect(&ctx).center() - moved.center()).length() <= 1.0);
        let _next = runtime.start(
            vec![PathBuf::from("source/other.txt")],
            PathBuf::from("target"),
        );
        settle(&ctx, &runtime, &mut panel, size);
        assert!(
            (rect(&ctx).center() - center).length() <= 1.0,
            "A new copy must open centered"
        );
    }
}
