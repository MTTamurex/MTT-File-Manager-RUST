use super::*;
use crate::app::copy_panel_state::CopyPanelState;
use crate::infrastructure::copy_engine::{CopyRuntime, Outcome};

fn texts(shape: &egui::epaint::Shape, result: &mut Vec<String>) {
    match shape {
        egui::epaint::Shape::Text(text) => result.push(text.galley.job.text.clone()),
        egui::epaint::Shape::Vec(shapes) => {
            for shape in shapes {
                texts(shape, result);
            }
        }
        _ => {}
    }
}

#[test]
fn deletion_uses_shared_window_without_copy_units_or_pause_controls() {
    for scale in [1.0, 2.0] {
        let ctx = egui::Context::default();
        crate::ui::interface_scale::apply(&ctx, scale);
        let runtime = CopyRuntime::default();
        let job = runtime.start_delete(vec![PathBuf::from("source/folder")], true);
        job.operation
            .deletion
            .as_ref()
            .unwrap()
            .removed
            .store(12, Ordering::Relaxed);
        let mut panel = CopyPanelState::default();
        let mut labels = Vec::new();
        for _ in 0..8 {
            labels.clear();
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200.0, 1000.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    super::super::window::show(ui.ctx(), &runtime.jobs(), &mut panel);
                },
            );
            for shape in output.shapes {
                texts(&shape.shape, &mut labels);
            }
        }
        assert!(labels
            .iter()
            .any(|label| label == &t!("delete_engine.deleting")));
        assert!(labels
            .iter()
            .any(|label| label == &t!("copy_engine.cancel")));
        assert!(!labels
            .iter()
            .any(|label| label.contains("MiB") || label == &t!("copy_engine.pause")));
        assert!(!job.operation.progress.is_cancelled());
        job.finish(Outcome {
            kind: OutcomeKind::Cancelled,
            message: Some(t!("delete_engine.cancelled").to_string()),
        });
        let elapsed = job.operation.elapsed();
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            card(ui, &job.operation, &[]);
        });
        labels.clear();
        for shape in output.shapes {
            texts(&shape.shape, &mut labels);
        }
        assert!(labels
            .iter()
            .any(|label| label == &t!("copy_engine.dismiss")));
        assert_eq!(job.operation.elapsed(), elapsed);
    }
}

#[test]
fn mixed_title_keeps_removed_items_separate_from_copied_bytes() {
    let runtime = CopyRuntime::default();
    let deletion = runtime.start_delete(vec![PathBuf::from("source/folder")], false);
    deletion
        .operation
        .deletion
        .as_ref()
        .unwrap()
        .removed
        .store(7, Ordering::Relaxed);
    let copy = runtime.start(
        vec![PathBuf::from("source/file.txt")],
        PathBuf::from("target"),
    );
    copy.operation
        .progress
        .copying
        .store(true, Ordering::Relaxed);
    copy.operation
        .progress
        .total_bytes
        .store(100, Ordering::Relaxed);
    copy.operation
        .progress
        .bytes_copied
        .store(25, Ordering::Relaxed);
    let label =
        super::super::summary::Summary::from_jobs(&runtime.jobs(), &CopyPanelState::default())
            .unwrap()
            .label();
    assert!(label.starts_with("25%"));
    assert!(label.contains(&t!("delete_engine.header", count = 7).to_string()));
    assert!(
        super::super::indicator::CopyIndicator::from_runtime(&runtime)
            .unwrap()
            .fraction
            .is_none()
    );
}

#[test]
fn confirmation_is_inline_and_enter_or_escape_cancels_by_default() {
    use crate::infrastructure::delete_engine::confirmation::{ask, tests::wait_for_request, Kind};
    for (scale, key) in [
        (1.0, egui::Key::Enter),
        (2.0, egui::Key::Enter),
        (1.0, egui::Key::Escape),
        (2.0, egui::Key::Escape),
    ] {
        let ctx = egui::Context::default();
        crate::ui::interface_scale::apply(&ctx, scale);
        let runtime = CopyRuntime::default();
        let job = runtime.start_delete(vec![PathBuf::from("source/file.txt")], true);
        let operation = job.operation.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let answer = ask(&operation, Kind::Permanent, &operation.sources);
            let _ = sender.send(answer);
            answer
        });
        wait_for_request(&job.operation);
        let mut panel = CopyPanelState::default();
        let mut draw = |events| {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200.0, 1000.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    super::super::window::show(ui.ctx(), &runtime.jobs(), &mut panel);
                },
            )
        };
        for _ in 0..8 {
            draw(vec![]);
        }
        let mut labels = Vec::new();
        for shape in draw(vec![]).shapes {
            texts(&shape.shape, &mut labels);
        }
        assert!(labels
            .iter()
            .any(|text| text == &t!("delete_engine.awaiting_confirmation")));
        assert!(labels
            .iter()
            .any(|text| text == &t!("delete_engine.confirm_permanent")));
        assert_eq!(
            labels
                .iter()
                .filter(|text| *text == &t!("copy_engine.cancel"))
                .count(),
            1
        );
        assert!(!labels.iter().any(|text| text == &t!("copy_engine.dismiss")));
        assert!(!worker.is_finished());
        draw(vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        let answer = receiver.recv_timeout(std::time::Duration::from_secs(3));
        if answer.is_err() {
            job.operation.progress.cancel();
        }
        assert_eq!(
            answer,
            Ok(false),
            "Confirmation did not cancel for {key:?} at scale {scale}"
        );
        assert!(!worker.join().unwrap());
        assert!(job.operation.progress.is_cancelled());
    }
}

#[test]
fn explicit_click_confirms_the_inline_fallback_without_cancelling_the_job() {
    use crate::infrastructure::delete_engine::confirmation::{ask, tests::wait_for_request, Kind};
    fn button_center(shape: &egui::epaint::Shape, label: &str) -> Option<egui::Pos2> {
        match shape {
            egui::epaint::Shape::Text(text) if text.galley.job.text == label => {
                Some(text.galley.rect.translate(text.pos.to_vec2()).center())
            }
            egui::epaint::Shape::Vec(shapes) => shapes
                .iter()
                .rev()
                .find_map(|shape| button_center(shape, label)),
            _ => None,
        }
    }
    for scale in [1.0, 2.0] {
        let ctx = egui::Context::default();
        crate::ui::interface_scale::apply(&ctx, scale);
        let runtime = CopyRuntime::default();
        let job = runtime.start_delete(vec![PathBuf::from("source/folder")], false);
        let operation = job.operation.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let answer = ask(&operation, Kind::RecycleUnavailable, &operation.sources);
            let _ = sender.send(answer);
            answer
        });
        wait_for_request(&job.operation);
        let mut panel = CopyPanelState::default();
        let mut draw = |events| {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200.0, 1000.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    super::super::window::show(ui.ctx(), &runtime.jobs(), &mut panel);
                },
            )
        };
        for _ in 0..8 {
            draw(vec![]);
        }
        let label = t!("delete_engine.confirm_permanent");
        let position = draw(vec![])
            .shapes
            .iter()
            .rev()
            .find_map(|shape| {
                button_center(&shape.shape, &label).filter(|pos| shape.clip_rect.contains(*pos))
            })
            .unwrap();
        let event = |pressed| {
            vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        draw(event(true));
        draw(event(false));
        let answer = receiver.recv_timeout(std::time::Duration::from_secs(3));
        if answer.is_err() {
            job.operation.progress.cancel();
        }
        assert_eq!(answer, Ok(true));
        assert!(worker.join().unwrap());
        assert!(!job.operation.progress.is_cancelled());
        assert!(job
            .operation
            .deletion
            .as_ref()
            .unwrap()
            .confirmation
            .pending()
            .is_none());
    }
}
