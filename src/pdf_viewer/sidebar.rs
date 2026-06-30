use eframe::egui;
use rust_i18n::t;

use super::viewer_app::PdfViewerApp;

const THUMB_MAX_W: f32 = 120.0;
const THUMB_MAX_H: f32 = 170.0;

impl PdfViewerApp {
    pub(super) fn show_sidebar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("pdf_thumbnail_sidebar")
            .resizable(true)
            .default_width(170.0)
            .width_range(140.0..=260.0)
            .show(ctx, |ui| {
                ui.heading(t!("pdfviewer.thumbnails_title").to_string());
                ui.separator();

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for idx in 0..self.total_pages {
                            self.show_thumbnail_item(ui, idx);
                            ui.add_space(8.0);
                        }
                    });
            });
    }

    fn show_thumbnail_item(&mut self, ui: &mut egui::Ui, idx: u32) {
        ui.push_id(idx, |ui| {
            let is_current = idx == self.current_page;
            let (display_w, display_h, render_w, render_h) = self.thumbnail_sizes(idx);
            let item_h = display_h + 24.0;
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), item_h),
                egui::Layout::top_down(egui::Align::Center),
                |ui| {
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(display_w, display_h),
                        egui::Sense::click(),
                    );

                    if response.clicked() {
                        self.go_to_page(idx);
                    }

                    if is_current && self.last_sidebar_scrolled_page != Some(idx) {
                        response.scroll_to_me(Some(egui::Align::Center));
                        self.last_sidebar_scrolled_page = Some(idx);
                    }

                    if ui.is_rect_visible(rect) {
                        self.submit_thumbnail(idx, render_w, render_h);
                        ui.painter().rect_filled(rect, 2.0, egui::Color32::WHITE);
                        if let Some(thumbnail) = self.thumbnail_textures.get(&idx) {
                            Self::paint_page(
                                ui.painter(),
                                rect,
                                thumbnail.texture.id(),
                                self.rotation,
                            );
                        } else {
                            ui.painter().rect_filled(
                                rect,
                                2.0,
                                if ui.visuals().dark_mode {
                                    egui::Color32::from_gray(46)
                                } else {
                                    egui::Color32::from_gray(220)
                                },
                            );
                            ui.painter().text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                t!("pdfviewer.thumbnail_loading").to_string(),
                                egui::FontId::proportional(11.0),
                                if ui.visuals().dark_mode {
                                    egui::Color32::from_gray(170)
                                } else {
                                    egui::Color32::from_gray(90)
                                },
                            );
                        }

                        let stroke_color = if is_current {
                            egui::Color32::from_rgb(80, 150, 255)
                        } else if ui.visuals().dark_mode {
                            egui::Color32::from_gray(90)
                        } else {
                            egui::Color32::from_gray(160)
                        };
                        ui.painter().rect_stroke(
                            rect.expand(1.0),
                            2.0,
                            egui::Stroke::new(if is_current { 2.0 } else { 1.0 }, stroke_color),
                            egui::StrokeKind::Outside,
                        );
                    }

                    let text = if is_current {
                        egui::RichText::new(t!("pdfviewer.page", page = idx + 1).to_string())
                            .strong()
                    } else {
                        egui::RichText::new(t!("pdfviewer.page", page = idx + 1).to_string())
                    };
                    if ui.selectable_label(is_current, text).clicked() {
                        self.go_to_page(idx);
                    }
                },
            );
        });
    }

    fn thumbnail_sizes(&self, page_idx: u32) -> (f32, f32, u32, u32) {
        let (natural_w, natural_h) = self.page_sizes[page_idx as usize];
        let (rotated_w, rotated_h) = if !self.rotation.is_multiple_of(180) {
            (natural_h, natural_w)
        } else {
            (natural_w, natural_h)
        };
        let scale = (THUMB_MAX_W / rotated_w).min(THUMB_MAX_H / rotated_h);
        let display_w = (rotated_w * scale).max(1.0);
        let display_h = (rotated_h * scale).max(1.0);
        let render_w = (natural_w * scale).max(1.0) as u32;
        let render_h = (natural_h * scale).max(1.0) as u32;
        (display_w, display_h, render_w, render_h)
    }
}
