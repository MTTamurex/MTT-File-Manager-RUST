use super::*;
use crate::app::copy_panel_state::CopyPanelState;
use crate::infrastructure::copy_engine::CopyRuntime;
use std::time::{Duration, Instant};

fn texts(shape: &egui::epaint::Shape, values: &mut Vec<String>) {
    match shape {
        egui::epaint::Shape::Text(text) => values.push(text.galley.job.text.clone()),
        egui::epaint::Shape::Vec(shapes) => {
            for shape in shapes {
                texts(shape, values);
            }
        }
        _ => {}
    }
}

#[test]
fn file_conflict_uses_app_window_and_keyboard_defaults_to_cancel() {
    for (scale, key) in [(1.0, egui::Key::Enter), (2.0, egui::Key::Escape)] {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        std::fs::create_dir(&source).unwrap();
        std::fs::create_dir(&target).unwrap();
        let file = source.join("file.txt");
        std::fs::write(&file, b"new").unwrap();
        std::fs::write(target.join("file.txt"), b"old").unwrap();
        let runtime = CopyRuntime::default();
        let job = runtime.start(vec![file], target.clone());
        let operation = job.operation.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let result = mtt_copy_engine::copy_with_conflicts(
                &operation.sources,
                &operation.destination,
                &operation.progress,
            );
            let _ = sender.send(result.is_err());
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while job.operation.progress.conflicts.pending().is_none() {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        let ctx = egui::Context::default();
        crate::ui::interface_scale::apply(&ctx, scale);
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
        for label in [
            t!("copy_engine.replace"),
            t!("copy_engine.skip"),
            t!("copy_engine.keep_both"),
            t!("copy_engine.apply_conflicts"),
        ] {
            assert!(labels.iter().any(|text| text == &label));
        }
        assert!(!labels.iter().any(|text| text == &t!("copy_engine.merge")));
        draw(vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        let answer = receiver.recv_timeout(Duration::from_secs(3));
        if answer.is_err() {
            job.operation.progress.cancel();
        }
        worker.join().unwrap();
        assert_eq!(answer, Ok(true));
        assert_eq!(std::fs::read(target.join("file.txt")).unwrap(), b"old");
        temp.close().unwrap();
    }
}

fn center(shape: &egui::epaint::Shape, label: &str) -> Option<egui::Pos2> {
    match shape {
        egui::epaint::Shape::Text(text) if text.galley.job.text == label => {
            Some(text.galley.rect.translate(text.pos.to_vec2()).center())
        }
        egui::epaint::Shape::Vec(shapes) => {
            shapes.iter().rev().find_map(|shape| center(shape, label))
        }
        _ => None,
    }
}

#[test]
fn explicit_replace_or_merge_and_apply_all_complete_without_repeated_dialogs() {
    for scale in [1.0, 2.0] {
        for directory in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let source = temp.path().join("source");
            let target = temp.path().join("target");
            std::fs::create_dir(&source).unwrap();
            std::fs::create_dir(&target).unwrap();
            let paths = if directory {
                for index in 0..2 {
                    let name = format!("nested-{index}");
                    std::fs::create_dir(source.join(&name)).unwrap();
                    std::fs::write(source.join(&name).join("file.txt"), b"new").unwrap();
                    std::fs::create_dir_all(target.join("source").join(&name)).unwrap();
                }
                std::fs::write(target.join("source/preserved.txt"), b"preserved").unwrap();
                vec![source.clone()]
            } else {
                (0..2)
                    .map(|index| {
                        let name = format!("file-{index}.txt");
                        std::fs::write(source.join(&name), b"new").unwrap();
                        std::fs::write(target.join(&name), b"old").unwrap();
                        source.join(name)
                    })
                    .collect()
            };
            let runtime = CopyRuntime::default();
            let job = runtime.start(paths, target.clone());
            let operation = job.operation.clone();
            let (sender, receiver) = std::sync::mpsc::channel();
            let worker = std::thread::spawn(move || {
                let result = mtt_copy_engine::copy_with_conflicts(
                    &operation.sources,
                    &operation.destination,
                    &operation.progress,
                )
                .map(|summary| (summary.files_copied, summary.failures));
                let _ = sender.send(result);
            });
            let deadline = Instant::now() + Duration::from_secs(5);
            while job.operation.progress.conflicts.pending().is_none() {
                assert!(Instant::now() < deadline);
                std::thread::yield_now();
            }
            let ctx = egui::Context::default();
            crate::ui::interface_scale::apply(&ctx, scale);
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
            let point = |output: &egui::FullOutput, label: &str| {
                output
                    .shapes
                    .iter()
                    .rev()
                    .find_map(|shape| {
                        center(&shape.shape, label).filter(|pos| shape.clip_rect.contains(*pos))
                    })
                    .expect("Conflict control must be visible")
            };
            let click = |position, pressed| {
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
            let position = point(&draw(vec![]), &t!("copy_engine.apply_conflicts"));
            draw(click(position, true));
            draw(click(position, false));
            let action = if directory {
                t!("copy_engine.merge")
            } else {
                t!("copy_engine.replace")
            };
            let position = point(&draw(vec![]), &action);
            draw(click(position, true));
            draw(click(position, false));
            let result = receiver.recv_timeout(Duration::from_secs(5));
            if result.is_err() {
                job.operation.progress.cancel();
            }
            worker.join().unwrap();
            assert_eq!(result.unwrap().unwrap(), (2, 0));
            if directory {
                assert_eq!(
                    std::fs::read(target.join("source/preserved.txt")).unwrap(),
                    b"preserved"
                );
                for index in 0..2 {
                    assert_eq!(
                        std::fs::read(target.join(format!("source/nested-{index}/file.txt")))
                            .unwrap(),
                        b"new"
                    );
                }
            } else {
                for index in 0..2 {
                    assert_eq!(
                        std::fs::read(target.join(format!("file-{index}.txt"))).unwrap(),
                        b"new"
                    );
                }
            }
            temp.close().unwrap();
        }
    }
}
