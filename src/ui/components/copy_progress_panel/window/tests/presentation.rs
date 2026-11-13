use super::*;

fn click(
    ctx: &egui::Context,
    runtime: &CopyRuntime,
    panel: &mut CopyPanelState,
    size: egui::Vec2,
    pos: egui::Pos2,
) {
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
                show(ui.ctx(), &runtime.jobs(), panel);
            },
        );
    }
}

#[test]
fn operation_actions_are_right_aligned_visible_and_still_work() {
    for scale in [1.0, 2.0] {
        for size in [egui::vec2(1200.0, 1000.0), egui::vec2(600.0, 1000.0)] {
            for deletion in [false, true] {
                let ctx = egui::Context::default();
                crate::ui::interface_scale::apply(&ctx, scale);
                let runtime = CopyRuntime::default();
                let paths = vec![PathBuf::from("source/file.txt")];
                let job = if deletion {
                    runtime.start_delete(paths, false)
                } else {
                    runtime.start(paths, PathBuf::from("target"))
                };
                let mut panel = CopyPanelState::default();
                for _ in 0..8 {
                    draw(&ctx, &runtime, &mut panel, size);
                }
                let texts = draw(&ctx, &runtime, &mut panel, size);
                let window = ctx
                    .memory(|memory| memory.area_rect(egui::Id::new(WINDOW_ID)))
                    .unwrap();
                let cancel = texts
                    .iter()
                    .find(|text| text.value == t!("copy_engine.cancel"))
                    .unwrap();
                assert!(cancel.clip.contains_rect(cancel.rect));
                assert!(ctx.viewport_rect().contains_rect(cancel.rect));
                assert!(
                    window.right() - cancel.rect.right() < 35.0,
                    "Cancel must be at the right edge: {:?}, {window:?}",
                    cancel.rect
                );
                if !deletion {
                    let pause = texts
                        .iter()
                        .find(|text| text.value == t!("copy_engine.pause"))
                        .unwrap();
                    assert!(pause.clip.contains_rect(pause.rect));
                    assert!(pause.rect.right() < cancel.rect.left());
                    assert!((pause.rect.center().y - cancel.rect.center().y).abs() < 1.0);
                    click(&ctx, &runtime, &mut panel, size, pause.rect.center());
                    assert_ne!(
                        job.operation.progress.pause_state(),
                        mtt_copy_engine::PauseState::Running
                    );
                    let texts = draw(&ctx, &runtime, &mut panel, size);
                    let resume = texts
                        .iter()
                        .find(|text| text.value == t!("copy_engine.resume"))
                        .unwrap();
                    click(&ctx, &runtime, &mut panel, size, resume.rect.center());
                    assert_eq!(
                        job.operation.progress.pause_state(),
                        mtt_copy_engine::PauseState::Running
                    );
                }
                let texts = draw(&ctx, &runtime, &mut panel, size);
                let cancel = texts
                    .iter()
                    .find(|text| text.value == t!("copy_engine.cancel"))
                    .unwrap();
                click(&ctx, &runtime, &mut panel, size, cancel.rect.center());
                assert!(job.operation.progress.is_cancelled());
                assert!(!panel.minimized);
            }
        }
    }
}

#[test]
fn window_presents_local_and_unc_paths_without_changing_operation_paths() {
    for (source, destination, shown_source, shown_destination) in [
        (
            r"\\?\C:\Documents\file.txt",
            r"\\?\D:\Documents",
            r"C:\Documents\file.txt",
            r"D:\Documents",
        ),
        (
            r"\\?\UNC\server\share\file.txt",
            r"\\?\UNC\server\target",
            r"\\server\share\file.txt",
            r"\\server\target",
        ),
    ] {
        let ctx = egui::Context::default();
        let runtime = CopyRuntime::default();
        let job = runtime.start(vec![PathBuf::from(source)], PathBuf::from(destination));
        let mut panel = CopyPanelState::default();
        let size = egui::vec2(1200.0, 1000.0);
        for _ in 0..8 {
            draw(&ctx, &runtime, &mut panel, size);
        }
        let texts = draw(&ctx, &runtime, &mut panel, size);
        assert!(texts.iter().any(|text| text.value == shown_source));
        assert!(texts.iter().any(|text| text.value == shown_destination));
        assert!(!texts.iter().any(|text| text.value.starts_with(r"\\?\")));
        assert_eq!(job.operation.sources, vec![PathBuf::from(source)]);
        assert_eq!(job.operation.destination, PathBuf::from(destination));
    }
}
