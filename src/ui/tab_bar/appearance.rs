use crate::domain::special_paths::{COMPUTER_VIEW_ID, RECYCLE_BIN_VIEW_ID};
use crate::infrastructure::onedrive::special_folder_display_name;
use eframe::egui::{self, Color32, CornerRadius};
use rust_i18n::t;
use std::path::Path;

pub(super) fn display_title_for_tab_path(path: &str, fallback_title: &str) -> String {
    if path == COMPUTER_VIEW_ID {
        t!("nav.computer").to_string()
    } else if path == RECYCLE_BIN_VIEW_ID {
        t!("nav.recycle_bin").to_string()
    } else {
        special_folder_display_name(Path::new(path)).unwrap_or_else(|| fallback_title.to_string())
    }
}

pub(super) fn paint_tab_background(ui: &egui::Ui, rect: egui::Rect, color: Color32) {
    const RADIUS: u8 = 8;
    ui.painter().rect_filled(
        rect,
        CornerRadius {
            nw: RADIUS,
            ne: RADIUS,
            sw: 0,
            se: 0,
        },
        color,
    );
    // A second opaque pass fills the bottom corners without double-blending translucent fills.
    if color.a() == u8::MAX {
        let body_top = (rect.min.y + RADIUS as f32 - 1.0).min(rect.max.y);
        let body = egui::Rect::from_min_max(egui::pos2(rect.min.x, body_top), rect.max);
        ui.painter().rect_filled(body, 0.0, color);
    }
}
