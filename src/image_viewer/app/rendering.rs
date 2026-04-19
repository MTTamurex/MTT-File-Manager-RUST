use crate::image_viewer::loader;
use eframe::egui;
use eframe::egui::scroll_area::ScrollBarVisibility;
use rust_i18n::t;

use super::{MAX_ZOOM_FACTOR, MIN_ZOOM_FACTOR};

impl super::DedicatedImageViewerApp {
    pub(super) fn render_top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("image_viewer_top_bar").show(ctx, |ui| {
            let mut selected_format = None;
            ui.horizontal_wrapped(|ui| {
                let total = self.sequence.entries.len();
                let prev_enabled = self.current_index > 0;
                let next_enabled = self.current_index + 1 < total;

                if ui
                    .add_enabled(prev_enabled, egui::Button::new(&*t!("imageviewer.previous")))
                    .clicked()
                {
                    self.navigate_prev(ctx);
                }

                if ui
                    .add_enabled(next_enabled, egui::Button::new(&*t!("imageviewer.next")))
                    .clicked()
                {
                    self.navigate_next(ctx);
                }

                ui.add_enabled_ui(!self.sequence.entries.is_empty() && !self.conversion_in_progress, |ui| {
                    ui.menu_button(t!("imageviewer.convert").to_string(), |ui| {
                        for format in loader::ExportImageFormat::ALL {
                            let label = Self::export_format_label(format);
                            if ui.button(label).clicked() {
                                selected_format = Some(format);
                            }
                        }
                    });
                });

                ui.separator();
                if total == 0 {
                    ui.label("0 / 0");
                } else {
                    ui.label(format!("{} / {}", self.current_index + 1, total));
                }
                ui.separator();
                let current_filename = self.current_filename();
                ui.label(current_filename.as_ref());
                if let Some(path) = self.current_path() {
                    ui.small(path.to_string_lossy());
                }
            });

            if let Some(format) = selected_format {
                self.start_conversion(format, ctx);
            }
        });
    }

    pub(super) fn sync_window_title(&self, ctx: &egui::Context) {
        let title = if self.sequence.entries.is_empty() {
            t!("imageviewer.title").to_string()
        } else {
            let current_filename = self.current_filename();
            t!("imageviewer.title_with_file", name = current_filename.as_ref()).to_string()
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
    }

    pub(super) fn render_bottom_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("image_viewer_bottom_bar").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(&*t!("imageviewer.zoom"));

                let mut slider_zoom = self.zoom_factor;
                let slider = egui::Slider::new(&mut slider_zoom, MIN_ZOOM_FACTOR..=MAX_ZOOM_FACTOR)
                    .show_value(false);

                if ui.add_sized([260.0, 20.0], slider).changed() {
                    self.zoom_factor = slider_zoom.clamp(MIN_ZOOM_FACTOR, MAX_ZOOM_FACTOR);
                }

                ui.label(format!("{:.0}%", self.zoom_percent_display.round()));

                ui.separator();
                if let Some((w, h)) = self.image_resolution {
                    ui.label(&*t!("imageviewer.resolution", w = w, h = h));
                } else {
                    ui.label(&*t!("imageviewer.resolution_none"));
                }

                if let Some(status) = &self.status_message {
                    ui.separator();
                    let color = if status.is_error {
                        egui::Color32::from_rgb(220, 80, 80)
                    } else {
                        egui::Color32::from_rgb(80, 170, 90)
                    };
                    ui.colored_label(color, &status.text);
                }
            });
        });
    }

    pub(super) fn render_center(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            // Prefer the current GIF animation frame; fall back to static texture.
            // Clone is cheap: egui::TextureHandle is reference-counted.
            let active_tex: Option<egui::TextureHandle> = if let Some(anim) = &self.gif_animation {
                anim.textures.get(anim.current_frame).cloned()
            } else {
                self.texture.clone()
            };

            if let Some(tex) = active_tex {
                // egui layout works in points, while texture size is in pixels.
                // Convert first to avoid implicit upscale on high-DPI monitors.
                let pixels_per_point = ui.ctx().pixels_per_point().max(f32::EPSILON);
                let tex_size = tex.size_vec2() / pixels_per_point;
                let viewport_size = ui.available_size();

                // Fit-to-window only downscales; 100% remains the native image
                // size, which keeps the reported zoom intuitive.
                let fit_scale = if tex_size.x <= 0.0 || tex_size.y <= 0.0 {
                    1.0
                } else {
                    let sx = viewport_size.x / tex_size.x;
                    let sy = viewport_size.y / tex_size.y;
                    sx.min(sy).min(1.0)
                };

                let draw_size = tex_size * fit_scale * self.zoom_factor;
                self.zoom_percent_display = fit_scale * self.zoom_factor * 100.0;

                let available_rect = ui.available_rect_before_wrap();
                let horizontal_scroll_bar_rect = egui::Rect::from_min_max(
                    egui::pos2(available_rect.left(), available_rect.bottom()),
                    egui::pos2(available_rect.right(), available_rect.bottom()),
                );

                egui::ScrollArea::both()
                    .id_salt("image_viewer_center_scroll")
                    .auto_shrink([false, false])
                    .scroll_bar_visibility(ScrollBarVisibility::VisibleWhenNeeded)
                    .scroll_bar_rect(horizontal_scroll_bar_rect)
                    .show(ui, |ui| {
                        let canvas_size = egui::vec2(
                            draw_size.x.max(viewport_size.x),
                            draw_size.y.max(viewport_size.y),
                        );
                        // Ensure content size is allowed to exceed viewport on both axes,
                        // so horizontal and vertical scrollbars can appear when needed.
                        ui.set_min_size(canvas_size);
                        let (canvas_rect, _) = ui.allocate_at_least(canvas_size, egui::Sense::hover());

                        let image = egui::Image::new(&tex)
                            .fit_to_exact_size(draw_size)
                            .sense(egui::Sense::click());
                        let image_rect = egui::Rect::from_center_size(canvas_rect.center(), draw_size);
                        let response = ui
                            .put(image_rect, image);

                        if response.hovered() {

                            let wheel_delta = ui.input(|i| i.raw_scroll_delta.y);
                            if wheel_delta.abs() > f32::EPSILON {
                                // Granular zoom: track wheel delta continuously.
                                let factor = 1.0 + wheel_delta * 0.0015;
                                if factor > 0.0 {
                                    self.zoom_factor = (self.zoom_factor * factor)
                                        .clamp(MIN_ZOOM_FACTOR, MAX_ZOOM_FACTOR);
                                }
                            }

                            let left_click =
                                ui.input(|i| i.pointer.button_clicked(egui::PointerButton::Primary));
                            if left_click {
                                self.zoom_factor =
                                    (self.zoom_factor * 1.25).clamp(MIN_ZOOM_FACTOR, MAX_ZOOM_FACTOR);
                            }

                            let right_click =
                                ui.input(|i| i.pointer.button_clicked(egui::PointerButton::Secondary));
                            if right_click {
                                self.zoom_factor =
                                    (self.zoom_factor / 1.25).clamp(MIN_ZOOM_FACTOR, MAX_ZOOM_FACTOR);
                            }
                        }
                    });
            } else if let Some(err) = &self.last_error {
                self.zoom_percent_display = 100.0;
                ui.with_layout(
                    egui::Layout::centered_and_justified(egui::Direction::TopDown),
                    |ui| {
                        ui.label(egui::RichText::new(err).color(egui::Color32::from_rgb(220, 80, 80)));
                    },
                );
            } else if self.sequence.entries.is_empty() {
                self.zoom_percent_display = 100.0;
                ui.with_layout(
                    egui::Layout::centered_and_justified(egui::Direction::TopDown),
                    |ui| {
                        ui.label(t!("imageviewer.no_image"));
                    },
                );
            } else {
                self.zoom_percent_display = 100.0;
                ui.with_layout(
                    egui::Layout::centered_and_justified(egui::Direction::TopDown),
                    |ui| {
                        let available = ui.available_size();
                        ui.allocate_response(available, egui::Sense::hover());
                    },
                );
            }
        });
    }
}
