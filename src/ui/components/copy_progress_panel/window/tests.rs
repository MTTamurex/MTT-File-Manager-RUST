use super::*;
use crate::infrastructure::copy_engine::CopyRuntime;
use std::path::PathBuf;

struct Text {
    value: String,
    rect: egui::Rect,
    clip: egui::Rect,
}

fn collect(shape: &egui::epaint::Shape, clip: egui::Rect, texts: &mut Vec<Text>) {
    match shape {
        egui::epaint::Shape::Text(text) => texts.push(Text {
            value: text.galley.job.text.clone(),
            rect: text.galley.rect.translate(text.pos.to_vec2()),
            clip,
        }),
        egui::epaint::Shape::Vec(shapes) => {
            for shape in shapes {
                collect(shape, clip, texts);
            }
        }
        _ => {}
    }
}

fn draw(
    ctx: &egui::Context,
    runtime: &CopyRuntime,
    panel: &mut CopyPanelState,
    size: egui::Vec2,
) -> Vec<Text> {
    let output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            ..Default::default()
        },
        |ui| {
            show(ui.ctx(), &runtime.jobs(), panel);
        },
    );
    let mut texts = Vec::new();
    for shape in output.shapes {
        collect(&shape.shape, shape.clip_rect, &mut texts);
    }
    texts
}

#[test]
fn full_window_keeps_the_minimize_row_and_copy_controls_visible() {
    for (size, scale) in [
        (egui::vec2(1200.0, 900.0), 1.0),
        (egui::vec2(1200.0, 900.0), 2.0),
        (egui::vec2(600.0, 1000.0), 1.0),
    ] {
        let ctx = egui::Context::default();
        crate::ui::interface_scale::apply(&ctx, scale);
        let runtime = CopyRuntime::default();
        let _job = runtime.start(
            vec![PathBuf::from("source/file.txt")],
            PathBuf::from("target"),
        );
        let mut panel = CopyPanelState::default();
        for _ in 0..8 {
            draw(&ctx, &runtime, &mut panel, size);
        }
        let texts = draw(&ctx, &runtime, &mut panel, size);
        let window = ctx
            .memory(|memory| memory.area_rect(egui::Id::new(WINDOW_ID)))
            .unwrap();
        let minimize = texts
            .iter()
            .find(|text| text.value == t!("copy_engine.minimize"))
            .unwrap();
        assert!(
            minimize.rect.top() - window.top() < 90.0,
            "Minimize row consumed the window height: {window:?}, {:?}",
            minimize.rect
        );
        for key in [
            "copy_engine.preparing",
            "copy_engine.elapsed",
            "copy_engine.remaining",
            "copy_engine.pause",
            "copy_engine.cancel",
        ] {
            let label = t!(key);
            let text = texts.iter().find(|text| text.value == label).unwrap();
            assert!(
                text.clip.contains_rect(text.rect),
                "Control is clipped: {label}"
            );
            assert!(
                ctx.viewport_rect().contains_rect(text.rect),
                "Control is outside viewport: {label}"
            );
        }
        assert!(
            window.height() <= ctx.viewport_rect().height(),
            "Window grows beyond the viewport: {window:?}"
        );
    }
}

#[test]
fn completed_window_shows_total_time_and_keeps_remaining_information() {
    use crate::infrastructure::copy_engine::{Outcome, OutcomeKind};
    let ctx = egui::Context::default();
    let runtime = CopyRuntime::default();
    let job = runtime.start(
        vec![PathBuf::from("source/file.txt")],
        PathBuf::from("target"),
    );
    job.finish(Outcome {
        kind: OutcomeKind::Completed,
        message: None,
    });
    let elapsed = job.operation.elapsed();
    let mut panel = CopyPanelState::default();
    let size = egui::vec2(1200.0, 900.0);
    for _ in 0..8 {
        draw(&ctx, &runtime, &mut panel, size);
    }
    let texts = draw(&ctx, &runtime, &mut panel, size);
    for key in [
        "copy_engine.total_time",
        "copy_engine.remaining",
        "copy_engine.dismiss",
    ] {
        let label = t!(key);
        let text = texts.iter().find(|text| text.value == label).unwrap();
        assert!(text.clip.contains_rect(text.rect), "Clipped label: {label}");
    }
    assert!(!texts
        .iter()
        .any(|text| text.value == t!("copy_engine.elapsed")));
    assert!(texts
        .iter()
        .any(|text| text.value == super::super::format_duration(elapsed)));
    assert_eq!(job.operation.elapsed(), elapsed);
}

#[test]
fn oversized_cached_geometry_does_not_hide_the_copy_controls() {
    let ctx = egui::Context::default();
    let size = egui::vec2(1200.0, 900.0);
    let _ = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            ..Default::default()
        },
        |ui| {
            // Reproduce old oversized content without clearing any window memory.
            egui::Window::new("Sizing fixture")
                .id(egui::Id::new(WINDOW_ID))
                .default_pos(egui::pos2(30.0, 30.0))
                .show(ui.ctx(), |ui| {
                    ui.allocate_space(egui::vec2(500.0, 2400.0));
                });
        },
    );
    assert!(
        ctx.memory(|memory| memory.area_rect(egui::Id::new(WINDOW_ID)))
            .unwrap()
            .height()
            > size.y
    );
    let runtime = CopyRuntime::default();
    let _job = runtime.start(
        vec![PathBuf::from("source/file.txt")],
        PathBuf::from("target"),
    );
    let mut panel = CopyPanelState::default();
    for _ in 0..8 {
        draw(&ctx, &runtime, &mut panel, size);
    }
    let texts = draw(&ctx, &runtime, &mut panel, size);
    let window = ctx
        .memory(|memory| memory.area_rect(egui::Id::new(WINDOW_ID)))
        .unwrap();
    assert!(window.height() <= size.y);
    for key in [
        "copy_engine.minimize",
        "copy_engine.pause",
        "copy_engine.cancel",
    ] {
        let label = t!(key);
        let text = texts.iter().find(|text| text.value == label).unwrap();
        assert!(
            text.clip.contains_rect(text.rect),
            "Control is clipped: {label}"
        );
        assert!(
            ctx.viewport_rect().contains_rect(text.rect),
            "Control is outside viewport: {label}"
        );
    }
}

#[test]
fn full_window_minimize_button_still_works_after_bounding_the_header() {
    let ctx = egui::Context::default();
    let size = egui::vec2(1200.0, 900.0);
    let runtime = CopyRuntime::default();
    let job = runtime.start(
        vec![PathBuf::from("source/file.txt")],
        PathBuf::from("target"),
    );
    let mut panel = CopyPanelState::default();
    for _ in 0..4 {
        draw(&ctx, &runtime, &mut panel, size);
    }
    let texts = draw(&ctx, &runtime, &mut panel, size);
    let pos = texts
        .iter()
        .find(|text| text.value == t!("copy_engine.minimize"))
        .unwrap()
        .rect
        .center();
    for pressed in [true, false] {
        let _ = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events: vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                ..Default::default()
            },
            |ui| {
                show(ui.ctx(), &runtime.jobs(), &mut panel);
            },
        );
    }
    assert!(panel.minimized);
    assert!(!job.operation.progress.is_cancelled());
    assert!(job.operation.outcome().is_none());
}
