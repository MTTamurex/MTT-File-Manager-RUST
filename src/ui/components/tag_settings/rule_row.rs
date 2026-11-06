use crate::ui::theme;
use eframe::egui;

fn galley(
    ui: &egui::Ui,
    text: &str,
    style: egui::TextStyle,
    color: egui::Color32,
    width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple(
        text.to_owned(),
        style.resolve(ui.style()),
        color,
        width.max(0.0),
    );
    job.wrap.max_rows = 1;
    ui.painter().layout_job(job)
}

/// Two independently truncated lines keep long paths from hiding the tag or
/// health indicator. A native button retains keyboard and focus behavior.
pub(super) fn show(
    ui: &mut egui::Ui,
    path: &str,
    tag: &str,
    status: &str,
    selected: bool,
) -> egui::Response {
    let main_height = ui.text_style_height(&egui::TextStyle::Button);
    let small_height = ui.text_style_height(&egui::TextStyle::Small);
    let padding = ui.spacing().button_padding;
    let height = main_height + small_height + padding.y * 2.0 + 4.0;
    let response = ui.add_sized(
        [ui.available_width(), height],
        egui::Button::new("")
            .selected(selected)
            .frame_when_inactive(false),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            format!("{path}, {tag}, {status}"),
        )
    });
    if ui.is_rect_visible(response.rect) {
        let rect = response.rect.shrink2(padding);
        let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
        let main = galley(
            ui,
            path,
            egui::TextStyle::Button,
            theme::text_color(ui.ctx()),
            rect.width(),
        );
        painter.galley(rect.min, main, theme::text_color(ui.ctx()));
        let color = theme::secondary_text_color(ui.ctx());
        let status = galley(
            ui,
            status,
            egui::TextStyle::Small,
            color,
            rect.width() * 0.45,
        );
        let tag = galley(
            ui,
            tag,
            egui::TextStyle::Small,
            color,
            (rect.width() - status.size().x - 8.0).max(0.0),
        );
        let y = rect.top() + main_height + 4.0;
        painter.galley(egui::pos2(rect.left(), y), tag, color);
        painter.galley(egui::pos2(rect.right() - status.size().x, y), status, color);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_paths_and_hierarchies_do_not_expand_compact_rows() {
        for width in [180.0, 320.0] {
            let ctx = egui::Context::default();
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.set_width(width);
                let response = show(
                    ui,
                    &"Folder / ".repeat(40),
                    &"Category / ".repeat(30),
                    "Watching",
                    true,
                );
                assert!(response.rect.width() <= width + 0.1);
                assert!(response.rect.height() < 90.0);
                let text = galley(
                    ui,
                    &"Category / ".repeat(30),
                    egui::TextStyle::Small,
                    egui::Color32::WHITE,
                    80.0,
                );
                assert_eq!(text.rows.len(), 1);
                assert!(text.size().x <= 80.1);
            });
        }
    }
}
