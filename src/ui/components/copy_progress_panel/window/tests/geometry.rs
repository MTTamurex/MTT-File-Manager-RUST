use super::*;
use crate::ui::components::copy_progress_panel::card_with_paths;

#[test]
fn file_transitions_keep_the_full_window_size_stable() {
    for scale in [1.0, 2.0] {
        let ctx = egui::Context::default();
        crate::ui::interface_scale::apply(&ctx, scale);
        let runtime = CopyRuntime::default();
        let job = runtime.start(
            vec![PathBuf::from("source/file.bin")],
            PathBuf::from("target"),
        );
        job.operation
            .progress
            .copying
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let mut panel = CopyPanelState::default();
        let mut draw_paths = |paths: &[PathBuf]| {
            for _ in 0..8 {
                let _ = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1200.0, 1000.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        show_with(
                            ui.ctx(),
                            &runtime.jobs(),
                            &mut panel,
                            |ui, operation, metrics| card_with_paths(ui, operation, metrics, paths),
                        );
                    },
                );
            }
            ctx.memory(|memory| memory.area_rect(egui::Id::new(WINDOW_ID)))
                .unwrap()
                .size()
        };
        let initial = draw_paths(&[]);
        let long = PathBuf::from(format!("source/{}.bin", "long-file-name-".repeat(30)));
        for paths in [
            vec![PathBuf::from("source/a.bin")],
            vec![long.clone()],
            vec![
                long.clone(),
                PathBuf::from("source/b.bin"),
                PathBuf::from("source/c.bin"),
            ],
            vec![
                long,
                PathBuf::from("source/b.bin"),
                PathBuf::from("source/c.bin"),
                PathBuf::from("source/d.bin"),
            ],
            vec![],
        ] {
            let actual = draw_paths(&paths);
            assert!(
                (actual.x - initial.x).abs() <= 1.0 && (actual.y - initial.y).abs() <= 1.0,
                "Window changed size at scale {scale}: {initial:?} -> {actual:?}"
            );
        }
    }
}

#[test]
fn changing_title_preserves_window_identity_and_user_width_without_duplicate_stats() {
    use crate::ui::components::copy_progress_panel::summary::Summary;
    use std::sync::atomic::Ordering;
    for (scale, size) in [
        (1.0, egui::vec2(1200.0, 1000.0)),
        (2.0, egui::vec2(1200.0, 1000.0)),
        (2.0, egui::vec2(600.0, 1000.0)),
    ] {
        let ctx = egui::Context::default();
        crate::ui::interface_scale::apply(&ctx, scale);
        // Seed a width different from the default, as retained after resizing.
        let _ = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                ..Default::default()
            },
            |ui| {
                egui::Window::new("Geometry fixture")
                    .id(egui::Id::new(WINDOW_ID))
                    .default_pos(egui::pos2(20.0, 70.0))
                    .default_width(560.0)
                    .show(ui.ctx(), |ui| {
                        ui.allocate_space(egui::vec2(540.0, 1.0));
                    });
            },
        );
        let runtime = CopyRuntime::default();
        let job = runtime.start(
            vec![PathBuf::from("source/file.bin")],
            PathBuf::from("target"),
        );
        let progress = &job.operation.progress;
        progress.copying.store(true, Ordering::Relaxed);
        progress.total_bytes.store(100, Ordering::Relaxed);
        progress.total_files.store(72, Ordering::Relaxed);
        let mut panel = CopyPanelState::default();
        let id = runtime.jobs()[0].0;
        panel.metrics.entry(id).or_default().speed = 185.0 * 1_048_576.0;
        for _ in 0..8 {
            draw(&ctx, &runtime, &mut panel, size);
        }
        let initial = ctx
            .memory(|memory| memory.area_rect(egui::Id::new(WINDOW_ID)))
            .unwrap();
        assert!(initial.width() > 520.0, "Retained user width was reset");
        progress.bytes_copied.store(42, Ordering::Relaxed);
        progress.files_copied.store(18, Ordering::Relaxed);
        for _ in 0..4 {
            draw(&ctx, &runtime, &mut panel, size);
        }
        let texts = draw(&ctx, &runtime, &mut panel, size);
        let expected = Summary::from_jobs(&runtime.jobs(), &panel).unwrap().label();
        assert!(expected.starts_with("42%"));
        let title = texts.iter().find(|text| text.value == expected).unwrap();
        assert!(title.clip.contains_rect(title.rect));
        assert!(!texts.iter().any(|text| text.value == "185.0 MiB/s"));
        assert!(!texts.iter().any(|text| text.value == "18 / 72"));
        let updated = ctx
            .memory(|memory| memory.area_rect(egui::Id::new(WINDOW_ID)))
            .unwrap();
        assert!((updated.min - initial.min).length() <= 1.0);
        assert!((updated.size() - initial.size()).length() <= 1.0);
        progress.total_bytes.store(u64::MAX, Ordering::Relaxed);
        progress.bytes_copied.store(u64::MAX / 2, Ordering::Relaxed);
        progress.total_files.store(usize::MAX, Ordering::Relaxed);
        progress
            .files_copied
            .store(usize::MAX / 2, Ordering::Relaxed);
        for _ in 0..4 {
            draw(&ctx, &runtime, &mut panel, size);
        }
        let large = ctx
            .memory(|memory| memory.area_rect(egui::Id::new(WINDOW_ID)))
            .unwrap();
        assert!(
            (large.size() - initial.size()).length() <= 1.0,
            "Large counters changed the window size: {initial:?} -> {large:?}"
        );
    }
}
