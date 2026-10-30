//! Layer-aware hit testing for the treemap's custom geometric hover handling.
use eframe::egui;

pub(super) fn tooltip_id() -> egui::Id {
    egui::Id::new("disk_analysis_tooltip")
}

pub(super) fn allows_pointer(ui: &egui::Ui, pos: egui::Pos2) -> bool {
    ui.ctx().layer_id_at(pos).is_none_or(|layer| {
        layer == ui.layer_id() || layer == egui::LayerId::new(egui::Order::Tooltip, tooltip_id())
    })
}

#[cfg(test)]
mod tests;
