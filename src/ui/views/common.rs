//! Common helper functions for views
//! Follows .cursorrules: single responsibility, < 300 lines

use crate::domain::file_entry::{archive_type_label, FileEntry};
use eframe::egui;
use rust_i18n::t;

/// Delay (in seconds) before showing a tooltip on hover.
pub const TOOLTIP_DELAY_SECS: f32 = 0.3;
pub const ITEM_DRAG_START_THRESHOLD: f32 = 5.0;

/// Aligns a paint rect to physical pixels to avoid shimmer on thin icon details
/// during smooth, subpixel scrolling.
pub fn snap_rect_to_physical_pixels(ctx: &egui::Context, rect: egui::Rect) -> egui::Rect {
    let pixels_per_point = ctx.pixels_per_point().max(1.0);
    let snap = |value: f32| (value * pixels_per_point).round() / pixels_per_point;
    let snap_size = |value: f32| (value * pixels_per_point).round().max(1.0) / pixels_per_point;

    egui::Rect::from_min_size(
        egui::pos2(snap(rect.min.x), snap(rect.min.y)),
        egui::vec2(snap_size(rect.width()), snap_size(rect.height())),
    )
}

/// Gets file type string for display
pub fn get_file_type_string(item: &FileEntry) -> String {
    if let Some(label) = archive_type_label(&item.name) {
        return label;
    }
    if item.is_dir {
        t!("file_types.folder").to_string()
    } else if let Some(ext) = item.path.extension() {
        let ext_str = ext.to_string_lossy().to_uppercase();
        if !ext_str.is_empty() {
            t!("file_info.file_generic", ext = ext_str).to_string()
        } else {
            t!("file_info.file_unknown").to_string()
        }
    } else {
        t!("file_info.file_unknown").to_string()
    }
}

pub fn should_start_item_drag(
    response_drag_started: bool,
    response_dragged: bool,
    pointer_button_down_on_item: bool,
    press_origin: Option<egui::Pos2>,
    pointer_pos: Option<egui::Pos2>,
) -> bool {
    if response_drag_started || response_dragged {
        return true;
    }

    if !pointer_button_down_on_item {
        return false;
    }

    match (press_origin, pointer_pos) {
        (Some(origin), Some(pos)) => origin.distance(pos) >= ITEM_DRAG_START_THRESHOLD,
        _ => false,
    }
}

#[derive(Clone, Copy)]
pub struct ViewportTracker {
    pub first_visible_index: usize,
    pub last_visible_index: usize,
    pub prefetch_rows: usize,
    pub columns: usize,
}

impl ViewportTracker {
    pub fn new() -> Self {
        Self {
            first_visible_index: 0,
            last_visible_index: 0,
            prefetch_rows: 2,
            columns: 1,
        }
    }

    pub fn get_prefetch_range(&self, total_items: usize) -> (usize, usize) {
        if total_items == 0 {
            return (0, 0);
        }
        let items_per_prefetch = self.prefetch_rows.saturating_mul(self.columns).max(1);
        let prefetch_start = self.first_visible_index.saturating_sub(items_per_prefetch);
        let last_visible = self.last_visible_index.min(total_items.saturating_sub(1));
        let prefetch_end = (last_visible + 1 + items_per_prefetch).min(total_items);
        (prefetch_start, prefetch_end)
    }

    pub fn is_visible(&self, index: usize) -> bool {
        index >= self.first_visible_index && index <= self.last_visible_index
    }
}

impl Default for ViewportTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::should_start_item_drag;
    use eframe::egui;

    #[test]
    fn explicit_drag_response_starts_item_drag() {
        assert!(should_start_item_drag(true, false, false, None, None));
        assert!(should_start_item_drag(false, true, false, None, None));
    }

    #[test]
    fn simple_click_jitter_does_not_start_item_drag() {
        assert!(!should_start_item_drag(
            false,
            false,
            true,
            Some(egui::pos2(10.0, 10.0)),
            Some(egui::pos2(13.0, 12.0)),
        ));
    }

    #[test]
    fn pointer_movement_past_threshold_starts_item_drag() {
        assert!(should_start_item_drag(
            false,
            false,
            true,
            Some(egui::pos2(10.0, 10.0)),
            Some(egui::pos2(16.0, 10.0)),
        ));
    }

    #[test]
    fn movement_without_button_down_does_not_start_item_drag() {
        assert!(!should_start_item_drag(
            false,
            false,
            false,
            Some(egui::pos2(10.0, 10.0)),
            Some(egui::pos2(30.0, 10.0)),
        ));
    }
}
