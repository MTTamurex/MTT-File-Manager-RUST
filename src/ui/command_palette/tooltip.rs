use eframe::egui;

const POINTER_GAP: f32 = 24.0;

fn pointer_anchor(pointer: egui::Pos2) -> egui::Rect {
    egui::Rect::from_min_max(
        pointer - egui::vec2(POINTER_GAP, 0.0),
        pointer + egui::vec2(POINTER_GAP, 0.0),
    )
}

pub(super) fn show(response: &egui::Response, text: &str) -> Option<egui::Rect> {
    if text.is_empty() {
        return None;
    }
    let pointer = response.ctx.input(|input| input.pointer.hover_pos())?;
    // Keep egui's normal delay and disabled-widget handling, but anchor beside
    // the pointer rather than underneath the entire command row.
    let mut tooltip = if response.enabled() {
        egui::Tooltip::for_enabled(response)
    } else {
        egui::Tooltip::for_disabled(response)
    };
    tooltip.popup = tooltip
        .popup
        .anchor(pointer_anchor(pointer))
        .align(egui::RectAlign::RIGHT_START)
        .align_alternatives(&[
            egui::RectAlign::RIGHT_END,
            egui::RectAlign::LEFT_START,
            egui::RectAlign::LEFT_END,
        ])
        .gap(0.0);
    tooltip
        .width(360.0)
        .show(|ui| {
            ui.add(egui::Label::new(text).wrap());
        })
        .map(|output| output.response.rect)
}

#[cfg(test)]
mod tests;
