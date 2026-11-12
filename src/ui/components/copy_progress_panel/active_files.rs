use eframe::egui;
use rust_i18n::t;
use std::path::{Path, PathBuf};

const VISIBLE_FILES: usize = 3;
const EXPANSION_ID: &str = "additional_active_files";

pub(super) fn render_paths(ui: &mut egui::Ui, paths: &[PathBuf], workers: usize) {
    let width = ui.available_width();
    let row_height = ui.text_style_height(&egui::TextStyle::Body);
    ui.add_space(6.0);
    ui.add_sized(
        [width, row_height],
        egui::Label::new(egui::RichText::new(t!("copy_engine.active_files")).weak())
            .truncate()
            .halign(egui::Align::Min),
    );
    let visible_files = workers.clamp(1, VISIBLE_FILES);
    // Reserve the worker budget (up to three rows) even between transfers. Filenames and
    // temporary empty snapshots must not change the window's desired size.
    for index in 0..visible_files {
        if let Some(path) = paths.get(index) {
            file_row(ui, path);
        } else {
            ui.allocate_space(egui::vec2(width, row_height));
        }
    }
    let id = ui.make_persistent_id(EXPANSION_ID);
    let mut expanded = ui.data(|data| data.get_temp::<bool>(id)).unwrap_or(false);
    let header_height = (ui.text_style_height(&egui::TextStyle::Button)
        + 2.0 * ui.spacing().button_padding.y)
        .max(ui.spacing().interact_size.y);
    if paths.len() > visible_files || expanded {
        let label = format!(
            "{} {}",
            if expanded { "▾" } else { "▸" },
            t!(
                "copy_engine.more_active_files",
                count = paths.len().saturating_sub(visible_files)
            )
        );
        // Expansion has its own id, independent of filename widget counts.
        let response = ui
            .allocate_ui_with_layout(
                egui::vec2(width, header_height),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| ui.add(egui::Button::new(label).frame(false).truncate()),
            )
            .inner;
        if response.clicked() {
            expanded = !expanded;
            ui.data_mut(|data| data.insert_temp(id, expanded));
            ui.ctx().request_repaint();
        }
    } else if workers > VISIBLE_FILES {
        // Reserve the expansion row before a fourth transfer arrives.
        ui.allocate_space(egui::vec2(width, header_height));
    }
    if expanded {
        egui::ScrollArea::vertical()
            .id_salt("active_files_scroll")
            .max_height(120.0)
            .min_scrolled_height(120.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for path in paths.iter().skip(visible_files) {
                    file_row(ui, path);
                }
            });
    }
}

fn file_row(ui: &mut egui::Ui, path: &Path) {
    let name = path
        .file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy();
    ui.add_sized(
        [
            ui.available_width(),
            ui.text_style_height(&egui::TextStyle::Body),
        ],
        egui::Label::new(name).truncate().halign(egui::Align::Min),
    )
    .on_hover_ui(|ui| {
        ui.label(path.display().to_string());
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(shape: &egui::epaint::Shape, result: &mut Vec<String>) {
        match shape {
            egui::epaint::Shape::Text(text) => result.push(text.galley.job.text.clone()),
            egui::epaint::Shape::Vec(shapes) => {
                for shape in shapes {
                    text(shape, result);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn large_active_snapshots_keep_the_list_compact() {
        let ctx = egui::Context::default();
        let paths: Vec<_> = (0..100)
            .map(|index| PathBuf::from(format!("folder/file-{index}.txt")))
            .collect();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(420.0);
            render_paths(ui, &paths, usize::MAX);
        });
        // A second frame also covers egui's initial sizing pass.
        output.shapes.extend(
            ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.set_width(420.0);
                render_paths(ui, &paths, usize::MAX);
            })
            .shapes,
        );
        let mut labels = Vec::new();
        for shape in output.shapes {
            text(&shape.shape, &mut labels);
        }
        for index in 0..3 {
            assert!(labels
                .iter()
                .any(|label| label == &format!("file-{index}.txt")));
        }
        assert!(!labels.iter().any(|label| label == "file-3.txt"));
        assert!(labels.iter().any(|label| label.contains("97")));
    }

    #[test]
    fn expanded_list_keeps_its_reserved_height_when_transfers_disappear() {
        for scale in [1.0, 2.0] {
            let ctx = egui::Context::default();
            crate::ui::interface_scale::apply(&ctx, scale);
            let paths: Vec<_> = (0..5)
                .map(|i| PathBuf::from(format!("source/file-{i}.bin")))
                .collect();
            let draw = |paths: &[PathBuf]| {
                let mut height = 0.0;
                for _ in 0..8 {
                    let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                        ui.set_width(420.0);
                        let id = ui.make_persistent_id(EXPANSION_ID);
                        ui.data_mut(|data| data.insert_temp(id, true));
                        let before = ui.cursor().top();
                        render_paths(ui, paths, usize::MAX);
                        height = ui.cursor().top() - before;
                    });
                }
                height
            };
            let expanded_height = draw(&paths);
            assert!(expanded_height >= 120.0);
            let empty_height = draw(&[]);
            assert!(
                (expanded_height - empty_height).abs() <= 1.0,
                "Expanded area changed size: {expanded_height} -> {empty_height}"
            );
        }
    }

    #[test]
    fn known_worker_budgets_keep_small_operations_compact_and_stable() {
        for scale in [1.0, 2.0] {
            let ctx = egui::Context::default();
            crate::ui::interface_scale::apply(&ctx, scale);
            let paths: Vec<_> = (0..4)
                .map(|i| PathBuf::from(format!("source/file-{i}.bin")))
                .collect();
            let measure = |workers, paths: &[PathBuf]| {
                let mut height = 0.0;
                let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                    ui.set_width(420.0);
                    let before = ui.cursor().top();
                    render_paths(ui, paths, workers);
                    height = ui.cursor().top() - before;
                });
                height
            };
            let mut previous = 0.0;
            for workers in [1, 2, 4] {
                let empty = measure(workers, &[]);
                let transferring = measure(workers, &paths[..workers]);
                assert!(
                    (empty - transferring).abs() <= 1.0,
                    "File transitions resized a {workers}-worker area: {empty} -> {transferring}"
                );
                assert!(empty > previous);
                previous = empty;
            }
        }
    }

    #[test]
    fn expansion_button_still_reacts_when_files_disappear_between_press_and_release() {
        fn center(shape: &egui::epaint::Shape) -> Option<egui::Pos2> {
            match shape {
                egui::epaint::Shape::Text(text) if text.galley.job.text.starts_with("▾ ") => {
                    Some(text.galley.rect.translate(text.pos.to_vec2()).center())
                }
                egui::epaint::Shape::Vec(shapes) => shapes.iter().find_map(center),
                _ => None,
            }
        }
        for scale in [1.0, 2.0] {
            let ctx = egui::Context::default();
            crate::ui::interface_scale::apply(&ctx, scale);
            let paths: Vec<_> = (0..4)
                .map(|i| PathBuf::from(format!("source/file-{i}.bin")))
                .collect();
            let draw = |paths: &[PathBuf], events, initialize| {
                let mut open = false;
                let output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(800.0, 600.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        ui.set_width(420.0);
                        let id = ui.make_persistent_id(EXPANSION_ID);
                        if initialize {
                            ui.data_mut(|data| data.insert_temp(id, true));
                        }
                        render_paths(ui, paths, 4);
                        open = ui.data(|data| data.get_temp::<bool>(id)).unwrap_or(false);
                    },
                );
                (output, open)
            };
            draw(&paths, vec![], true);
            for _ in 0..4 {
                draw(&paths, vec![], false);
            }
            let (output, _) = draw(&paths, vec![], false);
            let position = output
                .shapes
                .iter()
                .find_map(|shape| center(&shape.shape))
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
            assert!(draw(&paths, event(true), false).1);
            assert!(
                !draw(&[], event(false), false).1,
                "Expansion button lost its click at scale {scale}"
            );
        }
    }
}
