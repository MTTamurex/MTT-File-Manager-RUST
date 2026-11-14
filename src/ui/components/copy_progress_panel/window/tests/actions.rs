use super::*;
use crate::infrastructure::copy_engine::{Outcome, OutcomeKind};

#[test]
fn title_close_cancels_active_jobs_and_dismisses_only_when_all_are_finished() {
    for scale in [1.0, 2.0] {
        for kind in 0..3 {
            for completed in [false, true] {
                let ctx = egui::Context::default();
                crate::ui::interface_scale::apply(&ctx, scale);
                let size = egui::vec2(1200.0, 1000.0);
                let runtime = CopyRuntime::default();
                let paths = vec![PathBuf::from("source/file.txt")];
                let job = match kind {
                    0 => runtime.start(paths, PathBuf::from("target")),
                    1 => runtime.start_move(paths, PathBuf::from("target")),
                    _ => runtime.start_delete(paths, false),
                };
                let operation = job.operation.clone();
                if completed {
                    job.finish(Outcome {
                        kind: OutcomeKind::Completed,
                        message: None,
                    });
                }
                let mut panel = CopyPanelState::default();
                for _ in 0..8 {
                    draw(&ctx, &runtime, &mut panel, size);
                }
                let texts = draw(&ctx, &runtime, &mut panel, size);
                let close = ctx
                    .read_response(egui::Id::new(controls::CLOSE_ID))
                    .unwrap();
                let minimize = ctx
                    .read_response(egui::Id::new(controls::MINIMIZE_ID))
                    .unwrap();
                assert_eq!(close.rect.size(), minimize.rect.size());
                assert_eq!(close.rect.size(), egui::vec2(26.0, 20.0));
                assert!(close.rect.center().x > minimize.rect.center().x);
                assert!((close.rect.center().y - minimize.rect.center().y).abs() < 1.0);
                assert!(ctx.viewport_rect().contains_rect(close.rect));
                assert!(!texts
                    .iter()
                    .any(|text| text.value == t!("copy_engine.minimize")
                        || text.value == t!("copy_engine.dismiss")));
                let position = close.rect.center();
                let mut dismissed = Vec::new();
                for pressed in [true, false] {
                    let _ = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                            events: vec![
                                egui::Event::PointerMoved(position),
                                egui::Event::PointerButton {
                                    pos: position,
                                    button: egui::PointerButton::Primary,
                                    pressed,
                                    modifiers: egui::Modifiers::NONE,
                                },
                            ],
                            ..Default::default()
                        },
                        |ui| dismissed.extend(show(ui.ctx(), &runtime.jobs(), &mut panel)),
                    );
                }
                assert_eq!(operation.progress.is_cancelled(), !completed);
                if completed {
                    assert_eq!(dismissed, vec![runtime.jobs()[0].0]);
                    for id in dismissed {
                        runtime.dismiss(id);
                    }
                    assert!(runtime.jobs().is_empty());
                } else {
                    assert!(dismissed.is_empty());
                    assert!(!panel.minimized);
                    assert!(operation.outcome().is_none());
                    panel.restore();
                    assert!(!panel.minimized);
                    assert!(runtime.jobs().is_empty());
                    assert!(
                        super::super::super::indicator::CopyIndicator::from_runtime(&runtime)
                            .is_none()
                    );
                }
            }
        }
    }
}

#[test]
fn close_with_mixed_jobs_cancels_active_jobs_without_creating_minimized_entries() {
    let ctx = egui::Context::default();
    let runtime = CopyRuntime::default();
    let finished = runtime.start(
        vec![PathBuf::from("source/first.txt")],
        PathBuf::from("target"),
    );
    finished.finish(Outcome {
        kind: OutcomeKind::Completed,
        message: None,
    });
    let active = runtime.start_delete(vec![PathBuf::from("source/second.txt")], false);
    let moving = runtime.start_move(
        vec![PathBuf::from("source/third.txt")],
        PathBuf::from("target"),
    );
    let copying = runtime.start(
        vec![PathBuf::from("source/fourth.txt")],
        PathBuf::from("target"),
    );
    copying.operation.progress.pause();
    let mut panel = CopyPanelState::default();
    let size = egui::vec2(1200.0, 1000.0);
    for _ in 0..8 {
        draw(&ctx, &runtime, &mut panel, size);
    }
    draw(&ctx, &runtime, &mut panel, size);
    let position = ctx
        .read_response(egui::Id::new(controls::CLOSE_ID))
        .unwrap()
        .rect
        .center();
    for pressed in [true, false] {
        let _ = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events: vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                ..Default::default()
            },
            |ui| assert!(show(ui.ctx(), &runtime.jobs(), &mut panel).is_empty()),
        );
    }
    assert!(!panel.minimized);
    assert!(runtime.jobs().is_empty());
    assert!(active.operation.progress.is_cancelled());
    assert!(moving.operation.progress.is_cancelled());
    assert!(copying.operation.progress.is_cancelled());
    assert!(!finished.operation.progress.is_cancelled());
    assert!(active.operation.outcome().is_none());

    // Restoring cannot resurrect closed jobs as retained/minimized entries.
    panel.restore();
    let texts = draw(&ctx, &runtime, &mut panel, size);
    assert!(!texts
        .iter()
        .any(|text| text.value == "×" || text.value == t!("copy_engine.dismiss")));
    assert!(!panel.minimized);
    assert!(active.operation.progress.is_cancelled());
    assert!(super::super::super::indicator::CopyIndicator::from_runtime(&runtime).is_none());
}
