use eframe::egui;
use rust_i18n::t;

use super::viewer_app::PdfViewerApp;
use super::virtual_layout::VariableRows;

const THUMB_MAX_W: f32 = 120.0;
const THUMB_MAX_H: f32 = 170.0;

impl PdfViewerApp {
    pub(super) fn show_sidebar(&mut self, ctx: &egui::Context) {
        let panel = egui::SidePanel::left("pdf_thumbnail_sidebar")
            .resizable(true)
            .default_width(170.0)
            .width_range(140.0..=260.0)
            .show(ctx, |ui| {
                ui.heading(t!("pdfviewer.thumbnails_title").to_string());
                ui.separator();

                let rows = self.thumbnail_rows.take().unwrap_or_else(|| {
                    VariableRows::new(
                        (0..self.total_pages).map(|idx| {
                            let (_, display_h, _, _) = self.thumbnail_sizes(idx);
                            display_h + 24.0
                        }),
                        ui.spacing().item_spacing.y + 8.0,
                    )
                });
                let mut scroll_area = egui::ScrollArea::vertical().auto_shrink([false, false]);
                let pointer_over_sidebar = ui.rect_contains_pointer(ui.max_rect());
                let manual_sidebar_navigation = pointer_over_sidebar
                    && ui.input(|input| {
                        input.raw_scroll_delta.y.abs() > f32::EPSILON
                            || input.pointer.primary_down()
                    });
                if manual_sidebar_navigation {
                    self.last_sidebar_scrolled_page = Some(self.current_page);
                    self.thumbnail_keyboard_focus = true;
                }
                if self.current_page < self.total_pages
                    && self.last_sidebar_scrolled_page != Some(self.current_page)
                {
                    let viewport_height = ui.available_height();
                    if let Some(offset) = rows
                        .centered_scroll_offset(self.current_page as usize, ui.available_height())
                    {
                        let offset = offset.min((rows.total_height() - viewport_height).max(0.0));
                        let target_viewport = egui::Rect::from_min_max(
                            egui::pos2(0.0, offset),
                            egui::pos2(ui.available_width(), offset + viewport_height),
                        );
                        let landing_rows = rows.visible_range(target_viewport, 0);
                        for row in rows.visible_range(target_viewport, 1) {
                            let (_, _, render_w, render_h) = self.thumbnail_sizes(row as u32);
                            self.submit_thumbnail(row as u32, render_w, render_h);
                        }
                        let landing_ready = landing_rows.clone().all(|row| {
                            let page_idx = row as u32;
                            self.thumbnail_textures.contains_key(&page_idx)
                                || self.thumbnail_failed.contains(&page_idx)
                        });
                        if landing_ready {
                            scroll_area = scroll_area.vertical_scroll_offset(offset);
                            self.last_sidebar_scrolled_page = Some(self.current_page);
                        }
                    }
                }

                scroll_area.show_viewport(ui, |ui, viewport| {
                    let content_width = viewport.width();
                    ui.set_min_size(egui::vec2(content_width, rows.total_height()));
                    let origin = ui.max_rect().min;

                    for row in rows.visible_range(viewport, 1) {
                        let Some(top) = rows.top(row) else { continue };
                        let Some(height) = rows.height(row) else {
                            continue;
                        };
                        let rect = egui::Rect::from_min_size(
                            origin + egui::vec2(0.0, top),
                            egui::vec2(content_width, height),
                        );
                        let mut row_ui = ui.new_child(
                            egui::UiBuilder::new()
                                .id_salt(("pdf_thumbnail_row", row))
                                .max_rect(rect)
                                .layout(egui::Layout::top_down(egui::Align::Center)),
                        );
                        self.show_thumbnail_item(&mut row_ui, row as u32);
                    }
                });
                self.thumbnail_rows = Some(rows);
            });

        if ctx.input(|input| input.pointer.any_pressed()) {
            self.thumbnail_keyboard_focus = panel.response.contains_pointer();
        }
    }

    fn show_thumbnail_item(&mut self, ui: &mut egui::Ui, idx: u32) {
        ui.push_id(idx, |ui| {
            let is_current = idx == self.current_page;
            let (display_w, display_h, render_w, render_h) = self.thumbnail_sizes(idx);
            self.submit_thumbnail(idx, render_w, render_h);
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
                        self.thumbnail_keyboard_focus = true;
                        self.go_to_page(idx);
                    }

                    if ui.is_rect_visible(rect) {
                        ui.painter().rect_filled(rect, 2.0, egui::Color32::WHITE);
                        if let Some(thumbnail) = self.thumbnail_textures.get(&idx) {
                            Self::paint_page(
                                ui.painter(),
                                rect,
                                thumbnail.texture.id(),
                                self.rotation,
                            );
                        } else {
                            let failed = self.thumbnail_failed.contains(&idx);
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
                                if failed {
                                    t!("pdfviewer.thumbnail_unavailable").to_string()
                                } else {
                                    t!("pdfviewer.thumbnail_loading").to_string()
                                },
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
                        self.thumbnail_keyboard_focus = true;
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
