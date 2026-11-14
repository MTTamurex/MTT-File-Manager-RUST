use super::*;
use crate::infrastructure::copy_engine::{Outcome, OutcomeKind};
use crate::infrastructure::delete_engine::confirmation::{ask, Kind};
use std::time::{Duration, Instant};

#[test]
fn close_cancels_pending_copy_move_and_delete_confirmations_without_authorization() {
    for scale in [1.0, 2.0] {
        for kind in 0..3 {
            let root = tempfile::tempdir().unwrap();
            let source = root.path().join("source");
            let target = root.path().join("target");
            std::fs::create_dir(&source).unwrap();
            std::fs::create_dir(&target).unwrap();
            let file = source.join("file.txt");
            std::fs::write(&file, b"source").unwrap();
            std::fs::write(target.join("file.txt"), b"old target").unwrap();
            let runtime = CopyRuntime::default();
            let job = match kind {
                0 => runtime.start(vec![file.clone()], target.clone()),
                1 => runtime.start_move(vec![file.clone()], target.clone()),
                _ => runtime.start_delete(vec![file.clone()], true),
            };
            let operation = job.operation.clone();
            let (sender, receiver) = std::sync::mpsc::channel();
            let worker = std::thread::spawn(move || {
                let refused = if kind == 2 {
                    !ask(&operation, Kind::Permanent, &operation.sources)
                } else {
                    let result = if kind == 0 {
                        mtt_copy_engine::copy_with_conflicts(
                            &operation.sources,
                            &operation.destination,
                            &operation.progress,
                        )
                    } else {
                        mtt_copy_engine::move_with_conflicts(
                            &operation.sources,
                            &operation.destination,
                            &operation.progress,
                        )
                    };
                    matches!(result, Err(error) if error.kind() == std::io::ErrorKind::Interrupted)
                };
                let _ = sender.send(refused);
            });
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let pending = if let Some(deletion) = &job.operation.deletion {
                    deletion.confirmation.pending().is_some()
                } else {
                    job.operation.progress.conflicts.pending().is_some()
                };
                if pending {
                    break;
                }
                assert!(Instant::now() < deadline, "Confirmation was not published");
                std::thread::yield_now();
            }
            if kind != 2 {
                job.operation.progress.pause();
            }
            let ctx = egui::Context::default();
            crate::ui::interface_scale::apply(&ctx, scale);
            let size = egui::vec2(1200.0, 1000.0);
            let mut panel = CopyPanelState::default();
            for _ in 0..8 {
                draw(&ctx, &runtime, &mut panel, size);
            }
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
            assert!(
                super::super::super::indicator::CopyIndicator::from_runtime(&runtime).is_none()
            );
            assert!(job.operation.progress.is_cancelled());
            let answer = receiver.recv_timeout(Duration::from_secs(3));
            if answer.is_err() {
                job.operation.progress.cancel();
            }
            worker.join().unwrap();
            assert_eq!(answer, Ok(true));
            assert!(job.operation.progress.conflicts.pending().is_none());
            assert!(job
                .operation
                .deletion
                .as_ref()
                .is_none_or(|deletion| deletion.confirmation.pending().is_none()));
            job.finish(Outcome {
                kind: OutcomeKind::Cancelled,
                message: None,
            });
            assert!(!panel.minimized);
            assert!(runtime.jobs().is_empty());
            assert_eq!(std::fs::read(&file).unwrap(), b"source");
            assert_eq!(
                std::fs::read(target.join("file.txt")).unwrap(),
                b"old target"
            );
            root.close().unwrap();
        }
    }
}
