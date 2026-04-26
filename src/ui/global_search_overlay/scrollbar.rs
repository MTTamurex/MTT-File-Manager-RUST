use eframe::egui;

pub(super) const SCROLLBAR_WIDTH: f32 = 4.0;
pub(super) const SCROLLBAR_MIN_HANDLE: f32 = 30.0;
pub(super) const SCROLLBAR_GAP: f32 = 4.0;

#[derive(Clone, Copy, Debug)]
struct ScrollAnimationState {
    visual_scroll_y: f32,
}

pub(super) fn compute_visual_scroll(
    ui: &egui::Ui,
    target_scroll: f32,
    viewport_h: f32,
    generation: u64,
) -> (f32, f32) {
    let scroll_state_id = ui.id().with("global_search_scroll_state").with(generation);
    let dt = ui.input(|i| i.predicted_dt).min(0.05);

    let visual_scroll = ui.ctx().data_mut(|d| {
        let state = d.get_temp_mut_or_insert_with::<ScrollAnimationState>(scroll_state_id, || {
            ScrollAnimationState {
                visual_scroll_y: target_scroll,
            }
        });

        let t = (dt * 9.0).min(1.0);

        if (state.visual_scroll_y - target_scroll).abs() > viewport_h * 1.5 {
            state.visual_scroll_y = target_scroll;
        } else {
            state.visual_scroll_y =
                state.visual_scroll_y + (target_scroll - state.visual_scroll_y) * t;
        }

        if (state.visual_scroll_y - target_scroll).abs() < 1.0 {
            state.visual_scroll_y = target_scroll;
        }

        state.visual_scroll_y
    });

    let scroll_delta = (visual_scroll - target_scroll).abs();
    if scroll_delta > 0.5 {
        ui.ctx().request_repaint();
    }

    (visual_scroll, scroll_delta)
}

/// Custom scrollbar with track-click and drag (matches list view behavior).
pub(super) fn render_scrollbar(
    ui: &mut egui::Ui,
    viewport_rect: egui::Rect,
    viewport_h: f32,
    total_content_height: f32,
    max_scroll: f32,
    current_scroll: f32,
    scroll_offset: &mut f32,
) {
    let bar_rect = egui::Rect::from_min_max(
        egui::pos2(
            viewport_rect.right() - SCROLLBAR_WIDTH - 2.0,
            viewport_rect.top(),
        ),
        egui::pos2(viewport_rect.right() - 2.0, viewport_rect.bottom()),
    );

    let handle_h = (viewport_h / total_content_height * viewport_h)
        .max(SCROLLBAR_MIN_HANDLE)
        .min(viewport_h.max(SCROLLBAR_MIN_HANDLE));
    let travel = (viewport_h - handle_h).max(1.0);
    let handle_top = (current_scroll / max_scroll) * travel;
    let handle_rect = egui::Rect::from_min_size(
        egui::pos2(bar_rect.left(), viewport_rect.top() + handle_top),
        egui::vec2(SCROLLBAR_WIDTH, handle_h),
    );

    let scroll_id = ui.id().with("global_search_scrollbar");
    let response = ui.interact(bar_rect, scroll_id, egui::Sense::click_and_drag());

    if response.clicked() {
        if let Some(click_pos) = ui.input(|i| i.pointer.interact_pos()) {
            let relative_y = click_pos.y - bar_rect.top();
            let target_top = relative_y - (handle_h / 2.0);
            let ratio = target_top / travel;
            *scroll_offset = (ratio * max_scroll).clamp(0.0, max_scroll);
        }
    } else if response.dragged() {
        let delta = response.drag_delta().y;
        let scroll_per_pixel = max_scroll / travel;
        *scroll_offset += delta * scroll_per_pixel;
        *scroll_offset = scroll_offset.clamp(0.0, max_scroll);
    }

    // Draw track.
    ui.painter()
        .rect_filled(bar_rect, 0.0, egui::Color32::from_black_alpha(10));

    // Draw handle.
    let handle_color = if response.dragged() {
        egui::Color32::from_gray(100)
    } else if response.hovered() {
        egui::Color32::from_gray(150)
    } else {
        egui::Color32::from_gray(200)
    };
    ui.painter().rect_filled(handle_rect, 2.0, handle_color);
}
