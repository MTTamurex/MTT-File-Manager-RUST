//! Left sidebar of the disk analyzer: NTFS drives, file-type legend, stats.

use crate::app::disk_analysis_model::FileCategory;
use crate::app::state::ImageViewerApp;
use crate::infrastructure::windows::formatting::format_size;
use crate::ui::disk_analysis::category_color;
use eframe::egui;
use rust_i18n::t;

pub fn render_sidebar(app: &mut ImageViewerApp, ui: &mut egui::Ui) {
    ui.add_space(6.0);
    section_header(ui, &t!("disk_analysis.drives"));
    render_drives(app, ui);

    ui.add_space(10.0);
    ui.separator();
    ui.add_space(6.0);
    section_header(ui, &t!("disk_analysis.file_types"));
    render_categories(app, ui);

    if let Some(model) = app.disk_analysis.model.clone() {
        ui.add_space(10.0);
        ui.separator();
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(t!("disk_analysis.deepest_path").to_string())
                    .color(ui.visuals().weak_text_color()),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(model.deepest_path.to_string());
            });
        });
    }
}

fn section_header(ui: &mut egui::Ui, text: &str) {
    ui.label(
        egui::RichText::new(text.to_uppercase())
            .size(11.0)
            .color(ui.visuals().weak_text_color()),
    );
    ui.add_space(4.0);
}

fn render_drives(app: &mut ImageViewerApp, ui: &mut egui::Ui) {
    let disks = app.drive_state.disks.clone();
    let active_letter = app.disk_analysis.drive_letter;

    for (path, label) in disks {
        let letter = match path.chars().next() {
            Some(c) if c.is_ascii_alphabetic() => c.to_ascii_uppercase(),
            _ => continue,
        };
        let info = app.drive_state.cached_drive_info(&path);
        let is_ntfs = info
            .as_ref()
            .is_some_and(|i| i.file_system.eq_ignore_ascii_case("NTFS"));
        if !is_ntfs {
            continue;
        }
        let (total_space, free_space) = info
            .as_ref()
            .map(|i| (i.total_space, i.free_space))
            .unwrap_or((0, 0));
        let used = total_space.saturating_sub(free_space);
        let fraction = if total_space > 0 {
            used as f32 / total_space as f32
        } else {
            0.0
        };
        let is_active = active_letter == Some(letter);

        let response = ui.allocate_response(
            egui::vec2(ui.available_width(), 52.0),
            egui::Sense::click(),
        );
        let rect = response.rect;
        if is_active {
            ui.painter().rect_filled(
                rect.shrink(1.0),
                6.0,
                ui.visuals().widgets.active.weak_bg_fill,
            );
        } else if response.hovered() {
            ui.painter()
                .rect_filled(rect.shrink(1.0), 6.0, ui.visuals().widgets.hovered.weak_bg_fill);
        }

        let text_rect = rect.shrink2(egui::vec2(8.0, 6.0));
        ui.painter().text(
            text_rect.left_top(),
            egui::Align2::LEFT_TOP,
            format!("{}:  {}", letter, label),
            egui::FontId::proportional(13.0),
            ui.visuals().text_color(),
        );
        ui.painter().text(
            text_rect.right_top(),
            egui::Align2::RIGHT_TOP,
            format!("{:.0}%", fraction * 100.0),
            egui::FontId::proportional(12.0),
            ui.visuals().text_color(),
        );
        let bar_rect = egui::Rect::from_min_size(
            egui::pos2(text_rect.min.x, text_rect.min.y + 20.0),
            egui::vec2(text_rect.width(), 4.0),
        );
        ui.painter()
            .rect_filled(bar_rect, 2.0, ui.visuals().widgets.noninteractive.bg_fill);
        ui.painter().rect_filled(
            egui::Rect::from_min_size(
                bar_rect.min,
                egui::vec2(bar_rect.width() * fraction.clamp(0.0, 1.0), bar_rect.height()),
            ),
            2.0,
            ui.visuals().selection.bg_fill,
        );
        ui.painter().text(
            egui::pos2(text_rect.min.x, bar_rect.max.y + 3.0),
            egui::Align2::LEFT_TOP,
            t!(
                "disk_analysis.used_of",
                used = format_size(used),
                total = format_size(total_space)
            )
            .to_string(),
            egui::FontId::proportional(11.0),
            ui.visuals().weak_text_color(),
        );

        if response.clicked() && !is_active {
            app.disk_analysis.request(letter);
            ui.ctx().request_repaint();
        }
        ui.add_space(2.0);
    }
}

fn render_categories(app: &mut ImageViewerApp, ui: &mut egui::Ui) {
    let Some(model) = app.disk_analysis.model.clone() else {
        return;
    };
    let dark = ui.visuals().dark_mode;
    let total = model.total_size.max(1);

    for category in FileCategory::ALL {
        let i = category.index();
        let bytes = model.category_totals[i];
        if bytes == 0 {
            continue;
        }
        let percent = (bytes as f64 / total as f64) * 100.0;
        ui.horizontal(|ui| {
            let (dot_rect, _) =
                ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
            ui.painter()
                .circle_filled(dot_rect.center(), 5.0, category_color(category, dark));
            ui.add_space(4.0);
            ui.label(category_label(category));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(format!("{percent:.0}%"))
                        .color(ui.visuals().weak_text_color()),
                );
                ui.add_space(8.0);
                ui.label(egui::RichText::new(format_size(bytes)).strong());
            });
        });
        ui.add_space(2.0);
    }
}

fn category_label(category: FileCategory) -> egui::WidgetText {
    let key = match category {
        FileCategory::Video => "disk_analysis.category_video",
        FileCategory::Images => "disk_analysis.category_images",
        FileCategory::Audio => "disk_analysis.category_audio",
        FileCategory::Archives => "disk_analysis.category_archives",
        FileCategory::Code => "disk_analysis.category_code",
        FileCategory::Documents => "disk_analysis.category_documents",
        FileCategory::System => "disk_analysis.category_system",
        FileCategory::Other => "disk_analysis.category_other",
    };
    t!(key).into()
}
