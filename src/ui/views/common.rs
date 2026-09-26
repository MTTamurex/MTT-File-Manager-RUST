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

/// Unsmoothed mouse-wheel delta for one frame, replicating egui's wheel
/// conversion (`WheelState::on_wheel_event`) so that scrolling with
/// interpolation disabled keeps the same speed and modifier behavior as the
/// smoothed path: `Line`/`Page` units are converted to points, Shift scrolls
/// horizontally, Alt forces vertical, and Ctrl/Cmd is zoom (no scroll).
pub fn raw_wheel_delta(input: &egui::InputState, viewport_height: f32) -> egui::Vec2 {
    // egui `InputOptions` native default; not publicly readable from Context.
    const LINE_SCROLL_SPEED: f32 = 40.0;

    let mut total = egui::Vec2::ZERO;
    for event in &input.events {
        let egui::Event::MouseWheel {
            unit,
            delta,
            phase: egui::TouchPhase::Move,
            modifiers,
        } = event
        else {
            continue;
        };
        // Ctrl/Cmd is egui's zoom modifier: the wheel zooms instead of scrolling.
        if modifiers.matches_any(egui::Modifiers::COMMAND) {
            continue;
        }
        let mut delta = match unit {
            egui::MouseWheelUnit::Point => *delta,
            egui::MouseWheelUnit::Line => *delta * LINE_SCROLL_SPEED,
            egui::MouseWheelUnit::Page => *delta * viewport_height,
        };
        let is_horizontal = modifiers.matches_any(egui::Modifiers::SHIFT);
        let is_vertical = modifiers.matches_any(egui::Modifiers::ALT);
        if is_horizontal && !is_vertical {
            delta = egui::vec2(delta.x + delta.y, 0.0);
        } else if !is_horizontal && is_vertical {
            delta = egui::vec2(0.0, delta.x + delta.y);
        }
        total += delta;
    }
    total
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
    primary_button_down: bool,
    pointer_button_down_on_item: bool,
    press_origin: Option<egui::Pos2>,
    pointer_pos: Option<egui::Pos2>,
) -> bool {
    if !primary_button_down {
        return false;
    }

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

/// A secondary click targets an item when it hits visible item content or any
/// part of a row that already belongs to the current selection.
pub fn secondary_click_targets_item(is_selected: bool, content_hit: bool) -> bool {
    is_selected || content_hit
}

pub fn effective_item_selection(
    committed_selection: bool,
    rectangle_preview: Option<bool>,
) -> bool {
    rectangle_preview.unwrap_or(committed_selection)
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
    use super::{effective_item_selection, secondary_click_targets_item, should_start_item_drag};
    use eframe::egui;

    #[test]
    fn explicit_drag_response_starts_item_drag() {
        assert!(should_start_item_drag(true, false, true, false, None, None));
        assert!(should_start_item_drag(false, true, true, false, None, None));
    }

    #[test]
    fn simple_click_jitter_does_not_start_item_drag() {
        assert!(!should_start_item_drag(
            false,
            false,
            true,
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
            false,
            Some(egui::pos2(10.0, 10.0)),
            Some(egui::pos2(30.0, 10.0)),
        ));
    }

    #[test]
    fn side_button_drag_does_not_start_item_drag() {
        assert!(!should_start_item_drag(true, true, false, true, None, None));
        assert!(!should_start_item_drag(
            false,
            false,
            false,
            true,
            Some(egui::pos2(10.0, 10.0)),
            Some(egui::pos2(30.0, 10.0)),
        ));
    }

    #[test]
    fn egui_side_button_drag_is_not_treated_as_primary_drag() {
        use std::cell::Cell;

        fn observe_side_button_drag(button: egui::PointerButton) -> (bool, bool, bool, bool, bool) {
            fn render_drag_response(ui: &mut egui::Ui) -> egui::Response {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(100.0, 100.0), egui::Sense::hover());
                ui.interact(
                    rect,
                    egui::Id::new("side_button_drag_test"),
                    egui::Sense::click_and_drag(),
                )
            }

            let ctx = egui::Context::default();
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                let _ = render_drag_response(ui);
            });
            let _ = ctx.run_ui(
                egui::RawInput {
                    events: vec![egui::Event::PointerButton {
                        pos: egui::pos2(10.0, 10.0),
                        button,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    }],
                    ..Default::default()
                },
                |ui| {
                    let _ = render_drag_response(ui);
                },
            );

            let observed = Cell::new((false, false, false, false, false));
            let _ = ctx.run_ui(
                egui::RawInput {
                    events: vec![egui::Event::PointerMoved(egui::pos2(40.0, 10.0))],
                    ..Default::default()
                },
                |ui| {
                    let response = render_drag_response(ui);
                    let primary_down =
                        ui.input(|input| input.pointer.button_down(egui::PointerButton::Primary));
                    observed.set((
                        response.drag_started(),
                        response.drag_started_by(egui::PointerButton::Primary),
                        response.dragged(),
                        response.dragged_by(egui::PointerButton::Primary),
                        response.is_pointer_button_down_on(),
                    ));
                    assert!(!should_start_item_drag(
                        response.drag_started(),
                        response.dragged(),
                        primary_down,
                        response.is_pointer_button_down_on(),
                        ui.input(|input| input.pointer.press_origin()),
                        ui.input(|input| input.pointer.hover_pos()),
                    ));
                },
            );

            observed.get()
        }

        for button in [egui::PointerButton::Extra1, egui::PointerButton::Extra2] {
            let (drag_started, primary_drag_started, dragged, primary_dragged, button_down_on) =
                observe_side_button_drag(button);
            assert!(drag_started || dragged);
            assert!(!primary_drag_started);
            assert!(!primary_dragged);
            assert!(button_down_on);
        }
    }

    #[test]
    fn secondary_click_targets_content_or_selected_row() {
        assert!(!secondary_click_targets_item(false, false));
        assert!(secondary_click_targets_item(false, true));
        assert!(secondary_click_targets_item(true, false));
        assert!(secondary_click_targets_item(true, true));
    }

    #[test]
    fn rectangle_preview_is_the_effective_selection() {
        assert!(effective_item_selection(false, Some(true)));
        assert!(!effective_item_selection(true, Some(false)));
        assert!(effective_item_selection(true, None));
        assert!(!effective_item_selection(false, None));
    }
}
