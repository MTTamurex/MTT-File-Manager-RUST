use crate::infrastructure::windows::window_subclass::{
    clear_caption_drag_region, is_native_caption_drag_enabled, set_caption_drag_region_px,
};
use eframe::egui::{self, Color32, CornerRadius, Stroke, Vec2};

pub(super) fn render_new_tab_and_drag_area(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    tab_height: f32,
    window_controls_width: f32,
    inactive_bg: Color32,
    hover_bg: Color32,
    text_color: Color32,
) -> bool {
    let new_tab_btn_width = 36.0;
    let (new_tab_rect, new_tab_response) = ui.allocate_exact_size(
        Vec2::new(new_tab_btn_width, tab_height),
        egui::Sense::click(),
    );

    let new_tab_clicked = new_tab_response.clicked();

    let remaining_width = ui.available_width() - window_controls_width;
    if remaining_width > 0.0 {
        let native_caption_drag = is_native_caption_drag_enabled();
        let sense = if native_caption_drag {
            egui::Sense::hover()
        } else {
            egui::Sense::click_and_drag()
        };
        let (drag_rect, drag_response) = ui.allocate_exact_size(
            Vec2::new(remaining_width, tab_height),
            sense,
        );

        ui.painter().rect_filled(drag_rect, 0.0, inactive_bg);

        // Sync native caption drag area with this explicit empty strip.
        let ppp = ctx.pixels_per_point();
        set_caption_drag_region_px(
            (drag_rect.min.x * ppp).round() as i32,
            (drag_rect.min.y * ppp).round() as i32,
            (drag_rect.width() * ppp).round() as i32,
            (drag_rect.height() * ppp).round() as i32,
        );

        // Fallback path for environments where native caption drag is disabled.
        if !native_caption_drag && (drag_response.drag_started() || drag_response.dragged()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
    } else {
        clear_caption_drag_region();
    }

    // Keep "+" as a button, not as another tab.
    // Draw hover only to avoid visual seam with rounded tabs.
    if new_tab_response.hovered() {
        ui.painter()
            .rect_filled(new_tab_rect.shrink2(Vec2::splat(4.0)), CornerRadius::same(4), hover_bg);
    }

    let plus_center = new_tab_rect.center();
    let plus_size = 10.0;
    let plus_stroke = Stroke::new(1.0, text_color);
    ui.painter().line_segment(
        [
            plus_center + Vec2::new(-plus_size / 2.0, 0.0),
            plus_center + Vec2::new(plus_size / 2.0, 0.0),
        ],
        plus_stroke,
    );
    ui.painter().line_segment(
        [
            plus_center + Vec2::new(0.0, -plus_size / 2.0),
            plus_center + Vec2::new(0.0, plus_size / 2.0),
        ],
        plus_stroke,
    );

    new_tab_clicked
}
