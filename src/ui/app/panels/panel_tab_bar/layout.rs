use eframe::egui;

use super::{ADD_BUTTON_GAP, ADD_BUTTON_WIDTH, TAB_MAX_WIDTH, TAB_MIN_WIDTH};

pub(in super::super) fn panel_tab_strip_rect(
    panel: egui::Rect,
    strip_height: f32,
    border_reserved: f32,
) -> egui::Rect {
    let inset_x = border_reserved.min(panel.width() * 0.5);
    egui::Rect::from_min_max(
        panel.min + egui::vec2(inset_x, border_reserved.min(panel.height())),
        egui::pos2(
            panel.right() - inset_x,
            panel.top() + strip_height.min(panel.height()),
        ),
    )
}

pub(super) fn tab_strip_layout(rect: egui::Rect, tab_count: usize) -> (egui::Rect, f32) {
    let available = (rect.width() - ADD_BUTTON_WIDTH - ADD_BUTTON_GAP).max(0.0);
    let gaps = 2.0 * tab_count.saturating_sub(1) as f32;
    let tab_width =
        ((available - gaps) / tab_count.max(1) as f32).clamp(TAB_MIN_WIDTH, TAB_MAX_WIDTH);
    let width = (tab_width * tab_count as f32 + gaps).min(available);
    (
        egui::Rect::from_min_size(rect.min, egui::vec2(width, rect.height())),
        tab_width,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip(width: f32, count: usize) -> (egui::Rect, f32) {
        tab_strip_layout(
            egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(width, 36.0)),
            count,
        )
    }

    #[test]
    fn tabs_leave_focus_border_and_separator_drag_margin_clear() {
        let panel = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300.0, 600.0));
        let strip = panel_tab_strip_rect(panel, 36.0, 3.0);
        assert_eq!(strip.left(), panel.left() + 3.0);
        assert_eq!(strip.right(), panel.right() - 3.0);
        assert_eq!(strip.top(), panel.top() + 3.0);
        assert_eq!(strip.bottom(), panel.top() + 36.0);
    }

    #[test]
    fn tabs_stay_inside_a_short_or_narrow_panel() {
        let panel = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(4.0, 2.0));
        let strip = panel_tab_strip_rect(panel, 36.0, 3.0);
        assert!(panel.contains_rect(strip));
        assert!(strip.width() >= 0.0 && strip.height() >= 0.0);
    }

    #[test]
    fn add_button_follows_short_tab_list() {
        let (rect, width) = strip(600.0, 2);
        assert_eq!(width, TAB_MAX_WIDTH);
        assert_eq!(rect.width(), TAB_MAX_WIDTH * 2.0 + 2.0);
    }

    #[test]
    fn overflowing_tabs_reserve_space_for_add_button() {
        let (rect, width) = strip(300.0, 10);
        assert_eq!(width, TAB_MIN_WIDTH);
        assert_eq!(rect.width() + ADD_BUTTON_GAP + ADD_BUTTON_WIDTH, 300.0);
    }

    #[test]
    fn narrow_strip_never_has_negative_width() {
        assert_eq!(strip(10.0, 1).0.width(), 0.0);
    }

    #[test]
    fn rendered_tabs_fit_the_strip_at_fractional_widths() {
        let ctx = egui::Context::default();
        for scale in [1.0, 1.25, 1.5, 2.0] {
            ctx.set_pixels_per_point(scale);
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                for count in 2..=5 {
                    let (rect, tab_width) = strip(601.3, count);
                    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            let mut last = egui::Rect::NOTHING;
                            for _ in 0..count {
                                last = ui
                                    .allocate_exact_size(
                                        egui::vec2(tab_width, 28.0),
                                        egui::Sense::hover(),
                                    )
                                    .0;
                            }
                            assert!(
                                last.right() <= rect.right(),
                                "{count} tabs: {last:?} exceeds {rect:?}"
                            );
                        });
                    });
                }
            });
        }
    }
}
