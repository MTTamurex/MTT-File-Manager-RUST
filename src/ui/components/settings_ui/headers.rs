use crate::ui::theme;
use eframe::egui::{self, RichText};

/// A separated subsection within a settings page.
pub fn subsection_header(ui: &mut egui::Ui, title: &str, description: &str) {
    ui.add_space(8.0);
    ui.separator();
    ui.add_space(12.0);
    ui.label(
        RichText::new(title)
            .size(18.0)
            .strong()
            .color(theme::text_color(ui.ctx())),
    );
    if !description.is_empty() {
        ui.add_space(4.0);
        ui.label(
            RichText::new(description)
                .size(13.0)
                .color(theme::secondary_text_color(ui.ctx())),
        );
    }
    ui.add_space(12.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_headers_have_distinct_sizes_and_a_subsection_separator() {
        let ctx = egui::Context::default();
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            super::super::section_header(ui, "Page title", "Page description");
            subsection_header(ui, "Subsection title", "Subsection description");
        });
        let text_sizes: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some((
                    text.galley.job.text.as_str(),
                    text.galley.job.sections[0].format.font_id.size,
                )),
                _ => None,
            })
            .collect();
        assert!(text_sizes.contains(&("Page title", 22.0)));
        assert!(text_sizes.contains(&("Subsection title", 18.0)));
        assert!(text_sizes.contains(&("Page description", 13.0)));
        assert!(text_sizes.contains(&("Subsection description", 13.0)));
        assert!(output.shapes.iter().any(|shape| matches!(
            &shape.shape,
            egui::Shape::LineSegment { points, .. } if points[0].y == points[1].y
        )));
    }
}
