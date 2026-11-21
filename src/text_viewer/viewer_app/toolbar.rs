use super::TextViewerApp;
use eframe::egui;
use rust_i18n::t;

impl TextViewerApp {
    pub(super) fn show_toolbar(&mut self, ui: &mut egui::Ui) {
        let show_source_controls = !self.is_markdown
            || self.search_is_open()
            || self.goto_open
            || self.scroll_to_line.is_some();
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;

            let file_name = self
                .file_path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default();
            ui.label(egui::RichText::new(&file_name).strong().size(13.0));

            ui.separator();
            ui.label(
                egui::RichText::new(
                    t!(
                        "textviewer.line_count",
                        count = self.total_lines.to_string()
                    )
                    .to_string(),
                )
                .size(12.0),
            );

            ui.separator();
            ui.label(egui::RichText::new(format_file_size(self.file_size_bytes)).size(12.0));

            ui.separator();
            ui.label(egui::RichText::new(self.encoding_label).size(12.0));

            ui.separator();
            if ui
                .button(egui::RichText::new("A−").size(13.0))
                .on_hover_text(t!("textviewer.font_decrease"))
                .clicked()
            {
                self.font_size = (self.font_size - 1.0).max(super::MIN_FONT_SIZE);
            }
            ui.label(
                egui::RichText::new(format!("{:.0}px", self.font_size))
                    .size(12.0)
                    .monospace(),
            );
            if ui
                .button(egui::RichText::new("A+").size(13.0))
                .on_hover_text(t!("textviewer.font_increase"))
                .clicked()
            {
                self.font_size = (self.font_size + 1.0).min(super::MAX_FONT_SIZE);
            }

            if show_source_controls {
                ui.separator();
                let wrap_label = if self.word_wrap {
                    t!("textviewer.wrap_on")
                } else {
                    t!("textviewer.wrap_off")
                };
                if ui
                    .selectable_label(self.word_wrap, wrap_label.to_string())
                    .on_hover_text(t!("textviewer.wrap_toggle"))
                    .clicked()
                {
                    self.word_wrap = !self.word_wrap;
                }
            }

            ui.separator();
            let search_hint = if self.search_is_open() {
                t!("textviewer.search_close").to_string()
            } else {
                t!("textviewer.search_hint").to_string()
            };
            if ui
                .button(t!("textviewer.search_button").to_string())
                .on_hover_text(search_hint)
                .clicked()
            {
                self.toggle_search();
            }

            if ui
                .button(egui::RichText::new("Ln").size(12.0))
                .on_hover_text(t!("textviewer.goto_hint"))
                .clicked()
            {
                self.toggle_goto();
            }
        });
    }

    pub(super) fn show_goto_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.label(t!("textviewer.goto_label").to_string());

            let response = ui.add(
                egui::TextEdit::singleline(&mut self.goto_input)
                    .desired_width(100.0)
                    .hint_text(format!("1–{}", self.total_lines)),
            );

            if self.goto_focus_request {
                response.request_focus();
                self.goto_focus_request = false;
            }

            let go = ui.button("Go").clicked()
                || (response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)));
            if go {
                if let Ok(line_number) = self.goto_input.trim().parse::<usize>() {
                    if (1..=self.total_lines).contains(&line_number) {
                        self.scroll_to_line = Some(line_number - 1);
                    }
                }
                self.goto_open = false;
            }

            if ui.button("X").clicked() {
                self.goto_open = false;
            }
        });
    }
}

pub(super) fn top_bar_frame(ui: &egui::Ui) -> (egui::Frame, egui::Color32) {
    let (fill, separator) = crate::ui::theme::viewer_bar_colors(ui.ctx());
    (
        egui::Frame::new()
            .fill(fill)
            .inner_margin(egui::Margin::symmetric(10, 6)),
        separator,
    )
}

pub(super) fn paint_bar_separator(ui: &egui::Ui, separator: egui::Color32) {
    let inner = ui.max_rect();
    ui.painter().hline(
        inner.x_range(),
        inner.max.y + 6.0,
        egui::Stroke::new(1.0, separator),
    );
}

fn format_file_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}
