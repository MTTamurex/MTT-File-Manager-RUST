use super::TextViewerApp;
use eframe::egui;

impl TextViewerApp {
    pub(super) fn show_content(&mut self, ui: &mut egui::Ui) {
        if self.is_markdown
            && !self.search_is_open()
            && !self.goto_open
            && self.scroll_to_line.is_none()
        {
            self.show_markdown_content(ui);
            return;
        }

        let line_number_width =
            format!("{}", self.total_lines).len() as f32 * self.font_size * 0.6 + 16.0;
        let row_height = self.font_size + 4.0;
        let total_rows = self.total_lines;

        let scroll_id = egui::Id::new("text_viewer_content").with(&self.file_path);
        let mut scroll_area = egui::ScrollArea::both()
            .id_salt(scroll_id)
            .auto_shrink([false, false]);

        if let Some(target_line) = self.scroll_to_line.take() {
            let row_stride = row_height + ui.spacing().item_spacing.y;
            let y = target_line as f32 * row_stride;
            scroll_area = scroll_area.vertical_scroll_offset(y.max(0.0));
        }

        scroll_area.show_rows(ui, row_height, total_rows, |ui, row_range| {
            for line_idx in row_range {
                let line = self.line(line_idx);

                ui.horizontal(|ui| {
                    let line_num_text = format!(
                        "{:>width$}",
                        line_idx + 1,
                        width = format!("{}", self.total_lines).len()
                    );
                    ui.add_sized(
                        [line_number_width, row_height],
                        egui::Label::new(
                            egui::RichText::new(&line_num_text)
                                .monospace()
                                .size(self.font_size)
                                .color(ui.visuals().weak_text_color()),
                        ),
                    );

                    ui.separator();

                    self.show_searchable_line(ui, line_idx, line);
                });
            }
        });
    }
}
