use super::*;
use crate::infrastructure::copy_engine::{Outcome, OutcomeKind};

#[test]
fn title_close_hides_active_jobs_and_dismisses_only_when_all_are_finished() {
    for scale in [1.0, 2.0] {
        for deletion in [false, true] {
            for completed in [false, true] {
                let ctx = egui::Context::default();
                crate::ui::interface_scale::apply(&ctx, scale);
                let size = egui::vec2(1200.0, 1000.0);
                let runtime = CopyRuntime::default();
                let paths = vec![PathBuf::from("source/file.txt")];
                let job = if deletion {
                    runtime.start_delete(paths, false)
                } else {
                    runtime.start(paths, PathBuf::from("target"))
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
                let close = texts.iter().find(|text| text.value == "×").unwrap();
                let minimize = texts.iter().find(|text| text.value == "—").unwrap();
                assert!(close.rect.center().x > minimize.rect.center().x);
                assert!((close.rect.center().y - minimize.rect.center().y).abs() < 1.0);
                assert!(close.clip.contains_rect(close.rect));
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
                assert!(!operation.progress.is_cancelled());
                if completed {
                    assert_eq!(dismissed, vec![runtime.jobs()[0].0]);
                    for id in dismissed {
                        runtime.dismiss(id);
                    }
                    assert!(runtime.jobs().is_empty());
                } else {
                    assert!(dismissed.is_empty());
                    assert!(panel.minimized);
                    assert!(operation.outcome().is_none());
                    panel.restore();
                    assert!(!panel.minimized);
                    assert_eq!(runtime.jobs().len(), 1);
                }
            }
        }
    }
}

#[test]
fn close_with_mixed_jobs_keeps_finished_results_and_never_cancels_active_jobs() {
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
    let mut panel = CopyPanelState::default();
    let size = egui::vec2(1200.0, 1000.0);
    for _ in 0..8 {
        draw(&ctx, &runtime, &mut panel, size);
    }
    let texts = draw(&ctx, &runtime, &mut panel, size);
    let position = texts
        .iter()
        .find(|text| text.value == "×")
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
    assert!(panel.minimized);
    assert_eq!(runtime.jobs().len(), 2);
    assert!(!active.operation.progress.is_cancelled());
    assert!(active.operation.outcome().is_none());

    // A completed card can still be dismissed individually after restoring.
    panel.restore();
    let texts = draw(&ctx, &runtime, &mut panel, size);
    let position = texts
        .iter()
        .filter(|text| text.value == "×")
        .nth(1)
        .unwrap()
        .rect
        .center();
    let completed_id = runtime.jobs()[0].0;
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
    assert_eq!(dismissed, vec![completed_id]);
    assert!(!panel.minimized);
    assert!(!active.operation.progress.is_cancelled());
}
