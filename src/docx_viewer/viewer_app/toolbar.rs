use eframe::egui;
use rust_i18n::t;

use super::{DocxViewerApp, ZoomMode};

const MIN_ZOOM: f32 = 0.25;
const MAX_ZOOM: f32 = 4.0;

impl DocxViewerApp {
    pub(super) fn set_page(&mut self, page: usize) {
        if self.page_sizes.is_empty() {
            return;
        }
        self.current_page = page.min(self.page_sizes.len() - 1);
        self.scroll_to_page = Some(self.current_page);
    }

    pub(super) fn change_zoom(&mut self, multiplier: f32) {
        self.zoom = (self.effective_zoom * multiplier).clamp(MIN_ZOOM, MAX_ZOOM);
        self.zoom_mode = ZoomMode::Custom;
    }

    pub(super) fn current_scale(&self, available_width: f32, available_height: f32) -> f32 {
        let Some((width, height)) = self.page_sizes.get(self.current_page).copied() else {
            return self.zoom;
        };
        match self.zoom_mode {
            ZoomMode::FitWidth => (available_width / width).clamp(MIN_ZOOM, MAX_ZOOM),
            ZoomMode::FitPage => (available_width / width)
                .min(available_height / height)
                .clamp(MIN_ZOOM, MAX_ZOOM),
            ZoomMode::Custom => self.zoom,
        }
    }

    pub(super) fn handle_keyboard(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let (previous, next, first, last, zoom_in, zoom_out, reset) = ctx.input(|input| {
            let control = input.modifiers.ctrl || input.modifiers.command;
            (
                input.key_pressed(egui::Key::PageUp),
                input.key_pressed(egui::Key::PageDown),
                control && input.key_pressed(egui::Key::Home),
                control && input.key_pressed(egui::Key::End),
                control
                    && (input.key_pressed(egui::Key::Plus) || input.key_pressed(egui::Key::Equals)),
                control && input.key_pressed(egui::Key::Minus),
                control && input.key_pressed(egui::Key::Num0),
            )
        });

        if first {
            self.set_page(0);
        } else if last {
            self.set_page(self.page_sizes.len().saturating_sub(1));
        } else if previous {
            self.set_page(self.current_page.saturating_sub(1));
        } else if next {
            self.set_page(self.current_page.saturating_add(1));
        } else if zoom_in {
            self.change_zoom(1.2);
        } else if zoom_out {
            self.change_zoom(1.0 / 1.2);
        } else if reset {
            self.zoom_mode = ZoomMode::FitWidth;
        }
    }

    fn show_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let filename = self
                .file_path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default();
            ui.label(egui::RichText::new(filename).strong().size(13.0));
            ui.separator();

            if ui
                .button("‹")
                .on_hover_text(t!("docxviewer.previous_page").to_string())
                .clicked()
            {
                self.set_page(self.current_page.saturating_sub(1));
            }
            if ui
                .button("›")
                .on_hover_text(t!("docxviewer.next_page").to_string())
                .clicked()
            {
                self.set_page(self.current_page.saturating_add(1));
            }
            if !self.page_sizes.is_empty() {
                ui.label(
                    t!(
                        "docxviewer.page_count",
                        current = (self.current_page + 1).to_string(),
                        total = self.page_sizes.len().to_string()
                    )
                    .to_string(),
                );
            }

            ui.separator();
            if ui
                .button("−")
                .on_hover_text(t!("docxviewer.zoom_out").to_string())
                .clicked()
            {
                self.change_zoom(1.0 / 1.2);
            }
            ui.label(format!("{:.0}%", self.effective_zoom * 100.0));
            if ui
                .button("+")
                .on_hover_text(t!("docxviewer.zoom_in").to_string())
                .clicked()
            {
                self.change_zoom(1.2);
            }
            if ui
                .selectable_label(
                    self.zoom_mode == ZoomMode::FitWidth,
                    t!("docxviewer.fit_width"),
                )
                .clicked()
            {
                self.zoom_mode = ZoomMode::FitWidth;
            }
            if ui
                .selectable_label(
                    self.zoom_mode == ZoomMode::FitPage,
                    t!("docxviewer.fit_page"),
                )
                .clicked()
            {
                self.zoom_mode = ZoomMode::FitPage;
            }
        });
    }

    pub(super) fn show_toolbar_frame(&mut self, ui: &mut egui::Ui) {
        let (bar_fill, separator) = crate::ui::theme::viewer_bar_colors(ui.visuals().dark_mode);
        egui::Panel::top("docx_toolbar")
            .frame(
                egui::Frame::new()
                    .fill(bar_fill)
                    .inner_margin(egui::Margin::symmetric(10, 6)),
            )
            .show(ui, |ui| {
                self.show_toolbar(ui);
                let inner = ui.max_rect();
                ui.painter().hline(
                    inner.x_range(),
                    inner.max.y + 6.0,
                    egui::Stroke::new(1.0, separator),
                );
            });
    }
}
