use eframe::egui;
use rust_i18n::t;
use std::path::{Path, PathBuf};

const VISIBLE_FILES: usize = 3;

pub(super) fn render(ui: &mut egui::Ui, progress: &mtt_copy_engine::Progress) {
    render_paths(ui, &progress.active_files());
}

fn render_paths(ui: &mut egui::Ui, paths: &[PathBuf]) {
    if paths.is_empty() {
        return;
    }
    ui.add_space(6.0);
    ui.weak(t!("copy_engine.active_files"));
    for path in paths.iter().take(VISIBLE_FILES) {
        file_row(ui, path);
    }
    if paths.len() > VISIBLE_FILES {
        egui::CollapsingHeader::new(t!(
            "copy_engine.more_active_files",
            count = paths.len() - VISIBLE_FILES
        ))
        .id_salt("additional_active_files")
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("active_files_scroll")
                .max_height(120.0)
                .show(ui, |ui| {
                    for path in paths.iter().skip(VISIBLE_FILES) {
                        file_row(ui, path);
                    }
                });
        });
    }
}

fn file_row(ui: &mut egui::Ui, path: &Path) {
    let name = path
        .file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy();
    ui.add(egui::Label::new(name).truncate()).on_hover_ui(|ui| {
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
            render_paths(ui, &paths);
        });
        // A second frame also covers egui's initial sizing pass.
        output.shapes.extend(
            ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.set_width(420.0);
                render_paths(ui, &paths);
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
}
