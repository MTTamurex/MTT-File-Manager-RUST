use super::*;
use std::path::PathBuf;

#[test]
fn windows_path_keys_match_case_separators_and_extended_prefixes() {
    assert_eq!(
        path_key(Path::new(r"C:\Folder\file.txt")),
        path_key(Path::new("c:/folder/FILE.txt"))
    );
    assert_eq!(
        path_key(Path::new(r"\\?\C:\Folder\file.txt")),
        path_key(Path::new(r"C:\Folder\file.txt"))
    );
    assert_eq!(
        path_key(Path::new(r"\\?\UNC\server\share\file.txt")),
        path_key(Path::new(r"\\server\share\file.txt"))
    );
}

#[test]
fn copied_items_keep_full_icon_opacity_and_match_only_exact_paths() {
    let ctx = egui::Context::default();
    publish(
        &ctx,
        &[PathBuf::from(r"C:\Folder")],
        Some(ClipboardOp::Copy),
    );
    assert_eq!(
        operation(&ctx, Path::new(r"c:\folder")),
        Some(ClipboardOp::Copy)
    );
    assert_eq!(operation(&ctx, Path::new(r"C:\Folder\file.txt")), None);
    assert_eq!(icon_opacity(&ctx, Path::new(r"C:\Folder")), 1.0);
}

#[test]
fn cut_items_are_dimmed_and_replacement_removes_old_markers() {
    let ctx = egui::Context::default();
    let file = Path::new(r"C:\Folder\file.txt");
    publish(&ctx, &[file.to_path_buf()], Some(ClipboardOp::Move));
    assert_eq!(operation(&ctx, file), Some(ClipboardOp::Move));
    assert_eq!(icon_opacity(&ctx, file), 0.5);
    publish(
        &ctx,
        &[PathBuf::from(r"C:\Folder\other.txt")],
        Some(ClipboardOp::Copy),
    );
    assert_eq!(operation(&ctx, file), None);
    assert_eq!(icon_opacity(&ctx, file), 1.0);
    publish(&ctx, &[], None);
    assert_eq!(operation(&ctx, Path::new(r"C:\Folder\other.txt")), None);
}

#[test]
fn changed_sequence_clears_markers_and_rejects_delayed_results() {
    let ctx = egui::Context::default();
    let mut monitor = Monitor::default();
    let file = PathBuf::from(r"C:\Folder\file.txt");
    monitor.observe(&ctx, Some(10), None);
    monitor.apply_external(
        &ctx,
        FileSnapshot {
            sequence: 10,
            paths: vec![file.clone()],
            operation: ClipboardFileOp::Move,
        },
    );
    assert_eq!(operation(&ctx, &file), Some(ClipboardOp::Move));
    assert!(monitor.loaded);
    monitor.observe(&ctx, Some(11), None);
    assert_eq!(operation(&ctx, &file), None);
    monitor.apply_external(
        &ctx,
        FileSnapshot {
            sequence: 10,
            paths: vec![file.clone()],
            operation: ClipboardFileOp::Copy,
        },
    );
    assert_eq!(operation(&ctx, &file), None);
    assert!(!monitor.loaded);
}

#[test]
fn internal_operation_takes_priority_over_worker_result() {
    let ctx = egui::Context::default();
    let mut monitor = Monitor::default();
    let file = PathBuf::from(r"C:\Folder\file.txt");
    monitor.observe(&ctx, Some(20), Some((ClipboardOp::Copy, 1)));
    publish(&ctx, std::slice::from_ref(&file), Some(ClipboardOp::Copy));
    monitor.apply_external(
        &ctx,
        FileSnapshot {
            sequence: 20,
            paths: vec![file.clone()],
            operation: ClipboardFileOp::Move,
        },
    );
    assert_eq!(operation(&ctx, &file), Some(ClipboardOp::Copy));
}

#[test]
fn local_revision_invalidates_markers_even_with_unchanged_native_sequence() {
    let ctx = egui::Context::default();
    let mut monitor = Monitor::default();
    let file = PathBuf::from(r"C:\Folder\file.txt");
    monitor.observe(&ctx, Some(20), Some((ClipboardOp::Copy, 1)));
    publish(&ctx, std::slice::from_ref(&file), Some(ClipboardOp::Copy));
    monitor.loaded = true;
    monitor.observe(&ctx, Some(20), Some((ClipboardOp::Copy, 2)));
    assert!(!monitor.loaded);
    assert_eq!(operation(&ctx, &file), None);
}

#[test]
fn local_feedback_updates_in_one_pass_without_spawning_a_reader() {
    let ctx = egui::Context::default();
    let first = [PathBuf::from(r"C:\Folder\file.txt")];
    let second = [PathBuf::from(r"C:\Folder\other.txt")];
    update_for_sequence(&ctx, Some(40), Some((&first, ClipboardOp::Copy, 1)));
    assert_eq!(operation(&ctx, &first[0]), Some(ClipboardOp::Copy));
    update_for_sequence(&ctx, Some(40), Some((&second, ClipboardOp::Move, 2)));
    assert_eq!(operation(&ctx, &first[0]), None);
    assert_eq!(operation(&ctx, &second[0]), Some(ClipboardOp::Move));
    update_for_sequence(&ctx, Some(40), Some((&second, ClipboardOp::Move, 2)));
    assert_eq!(operation(&ctx, &second[0]), Some(ClipboardOp::Move));
    let monitor = ctx.data(|data| {
        data.get_temp::<Arc<Mutex<Monitor>>>(egui::Id::new("clipboard_monitor"))
            .unwrap()
    });
    assert!(monitor.lock().unwrap().pending.is_none());
}

#[test]
fn feedback_border_preserves_selection_in_light_and_dark_themes() {
    for dark in [false, true] {
        for operation in [ClipboardOp::Copy, ClipboardOp::Move] {
            let ctx = egui::Context::default();
            ctx.set_visuals(if dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            });
            let path = Path::new(r"C:\Folder\file.txt");
            publish(&ctx, &[path.to_path_buf()], Some(operation));
            let rect = egui::Rect::from_min_size(egui::pos2(10.0, 10.0), egui::vec2(160.0, 28.0));
            let selection = egui::Stroke::new(2.0, Color32::BLUE);
            let output = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.painter()
                    .rect_stroke(rect, 4.0, selection, egui::StrokeKind::Inside);
                paint(ui, rect, path);
            });
            let rectangles: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::epaint::Shape::Rect(rectangle) => Some(rectangle),
                    _ => None,
                })
                .collect();
            assert!(rectangles
                .iter()
                .any(|shape| shape.rect == rect && shape.stroke == selection));
            let feedback = rectangles
                .iter()
                .find(|shape| shape.rect == rect.shrink(3.0))
                .unwrap();
            let color = feedback.stroke.color;
            match operation {
                ClipboardOp::Copy => assert!(color.g() > color.r()),
                ClipboardOp::Move => assert!(color.r() > color.g() && color.g() > color.b()),
            }
        }
    }
}
