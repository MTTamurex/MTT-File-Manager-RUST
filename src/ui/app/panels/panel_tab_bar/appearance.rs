use eframe::egui;

use super::TAB_CORNER_RADIUS;

pub(super) fn layout_tab_title(
    ui: &egui::Ui,
    title: &str,
    font_id: egui::FontId,
    color: egui::Color32,
    max_width: f32,
) -> std::sync::Arc<egui::Galley> {
    let painter = ui.painter();
    let full_galley = painter.layout_no_wrap(title.to_owned(), font_id.clone(), color);
    if full_galley.size().x <= max_width {
        return full_galley;
    }

    let mut boundaries: Vec<usize> = title.char_indices().map(|(index, _)| index).collect();
    boundaries.push(title.len());
    let mut low = 0usize;
    let mut high = boundaries.len().saturating_sub(1);

    while low < high {
        let middle = (low + high).div_ceil(2);
        let mut candidate = title[..boundaries[middle]].to_owned();
        candidate.push('…');
        let galley = painter.layout_no_wrap(candidate, font_id.clone(), color);
        if galley.size().x <= max_width {
            low = middle;
        } else {
            high = middle - 1;
        }
    }

    let mut truncated = title[..boundaries[low]].to_owned();
    truncated.push('…');
    painter.layout_no_wrap(truncated, font_id, color)
}

pub(super) fn paint_tab_background(ui: &egui::Ui, rect: egui::Rect, color: egui::Color32) {
    ui.painter().rect_filled(
        rect,
        egui::CornerRadius {
            nw: TAB_CORNER_RADIUS,
            ne: TAB_CORNER_RADIUS,
            sw: 0,
            se: 0,
        },
        color,
    );

    if color.a() == u8::MAX {
        let body_top = (rect.min.y + TAB_CORNER_RADIUS as f32 - 1.0).min(rect.max.y);
        let body = egui::Rect::from_min_max(egui::pos2(rect.min.x, body_top), rect.max);
        ui.painter().rect_filled(body, 0.0, color);
    }
}
