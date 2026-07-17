use eframe::egui::{self, Color32, Rect, Sense, Ui};

const GRID_SCROLL_SPEED: f32 = 5.5;
const OPENGL_SCROLL_RESPONSE_RATE: f32 = 9.751_135;
const MAX_SCROLL_FRAME_DT: f32 = 0.05;

#[derive(Clone, Copy, Debug)]
struct ScrollState {
    visual_scroll_y: f32,
}

fn scroll_interpolation_factor(dt: f32, use_elapsed_time: bool) -> f32 {
    let dt = dt.clamp(0.0, MAX_SCROLL_FRAME_DT);
    if use_elapsed_time {
        1.0 - (-OPENGL_SCROLL_RESPONSE_RATE * dt).exp()
    } else {
        (dt * 9.0).min(1.0)
    }
}

pub(super) fn apply_scroll_input(
    ui: &Ui,
    target_scroll: &mut f32,
    max_scroll: f32,
    consume_scroll: bool,
) {
    let scroll_delta = if consume_scroll {
        ui.input(|i| i.smooth_scroll_delta.y)
    } else {
        0.0
    };

    if scroll_delta != 0.0 {
        *target_scroll += -scroll_delta * GRID_SCROLL_SPEED;
    }

    *target_scroll = target_scroll.clamp(0.0, max_scroll);
}

pub(super) fn compute_visual_scroll(
    ui: &Ui,
    target_scroll: f32,
    viewport_h: f32,
    generation: usize,
    is_opengl_backend: bool,
) -> (f32, f32) {
    // Scope scroll state to folder generation so visual_scroll resets on navigation
    let scroll_state_id = ui.id().with("scroll_state").with(generation);
    // Use predicted_dt (fixed, ~16.67ms) instead of stable_dt (variable).
    // stable_dt inherits latency spikes from eframe/wgpu (tessellation+present),
    // causing the lerp to "jump" on slow frames and "snap back" on following frames.
    // predicted_dt is constant and guarantees uniform visual motion.
    let dt = ui.input(|i| {
        if is_opengl_backend {
            i.stable_dt
        } else {
            i.predicted_dt
        }
    });

    let visual_scroll = ui.ctx().data_mut(|d| {
        let state = d.get_temp_mut_or_insert_with::<ScrollState>(scroll_state_id, || ScrollState {
            visual_scroll_y: target_scroll,
        });

        let t = scroll_interpolation_factor(dt, is_opengl_backend);

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
        // FIX: request_repaint() (immediate) instead of request_repaint_after(16ms).
        //
        // The 16ms timer is not synchronized with wgpu/driver vsync. When
        // the timer fires in the middle of a vsync cycle (e.g. 3ms before the next
        // present), eframe schedules update() too late -> it misses the
        // vsync window -> present() waits one more cycle (~16.7ms extra) -> effective dt
        // of ~33-42ms instead of ~16.7ms. This happens about every ~1 second,
        // creating the rhythmic micro-stutter pattern observed in the logs.
        //
        // request_repaint() asks for "as early as possible", and eframe synchronizes
        // naturally with vsync, eliminating the timing conflict.
        ui.ctx().request_repaint();
    }

    (visual_scroll, scroll_delta)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_custom_scrollbar(
    ui: &mut Ui,
    viewport_rect: Rect,
    viewport_h: f32,
    total_content_height: f32,
    current_scroll: f32,
    max_scroll: f32,
    target_scroll: &mut f32,
) {
    if total_content_height <= viewport_h || max_scroll <= 0.0 {
        return;
    }

    if viewport_h <= 0.0 {
        return;
    }

    // Floating scrollbar dimensions (matches egui ScrollStyle config)
    const RESTING_W: f32 = 3.0;
    const HOVER_W: f32 = 8.0;
    const MARGIN: f32 = 2.0;

    // Wide interaction zone so hover detection is easy
    let interact_rect = Rect::from_min_max(
        viewport_rect.right_top() - egui::vec2(HOVER_W + MARGIN * 2.0, 0.0),
        viewport_rect.right_bottom(),
    );

    let interact = ui.interact(
        interact_rect,
        ui.id().with("scrollbar"),
        Sense::click_and_drag(),
    );

    let is_hovered = interact.hovered() || interact.dragged();
    // Pointer anywhere in viewport = "active" state (show handle like egui's ScrollArea)
    let pointer_in_viewport = ui.input(|i| {
        i.pointer
            .hover_pos()
            .map(|p| viewport_rect.contains(p))
            .unwrap_or(false)
    });
    let bar_w = if is_hovered { HOVER_W } else { RESTING_W };

    let scrollbar_rect = Rect::from_min_max(
        egui::pos2(viewport_rect.right() - bar_w - MARGIN, viewport_rect.top()),
        egui::pos2(viewport_rect.right() - MARGIN, viewport_rect.bottom()),
    );

    let handle_h = (viewport_h / total_content_height * viewport_h)
        .max(30.0)
        .min(viewport_h.max(30.0));
    let travel = (viewport_h - handle_h).max(1.0);
    let handle_y = (current_scroll / max_scroll) * travel;
    let handle_rect = Rect::from_min_size(
        scrollbar_rect.min + egui::vec2(0.0, handle_y),
        egui::vec2(bar_w, handle_h),
    );

    if interact.clicked() {
        if let Some(click_pos) = ui.input(|i| i.pointer.interact_pos()) {
            let relative_y = click_pos.y - scrollbar_rect.top();
            let target_handle_top = relative_y - (handle_h / 2.0);
            let scroll_ratio = target_handle_top / travel;
            *target_scroll = (scroll_ratio * max_scroll).clamp(0.0, max_scroll);
        }
    } else if interact.dragged() {
        let delta_y = interact.drag_delta().y;
        let scroll_pct_delta = delta_y / travel;
        *target_scroll += scroll_pct_delta * max_scroll;
        *target_scroll = target_scroll.clamp(0.0, max_scroll);
    }

    // Track background — use egui's configured scroll style opacities
    let scroll_style = &ui.style().spacing.scroll;
    let bg_opacity = if interact.dragged() || interact.hovered() {
        scroll_style.interact_background_opacity
    } else {
        scroll_style.dormant_background_opacity
    };
    if bg_opacity > 0.0 {
        ui.painter().rect_filled(
            scrollbar_rect,
            4.0,
            Color32::from_black_alpha((bg_opacity * 255.0) as u8),
        );
    }

    // Handle — use egui's exact scroll handle color and opacity
    let handle_opacity = if interact.dragged() || is_hovered {
        scroll_style.interact_handle_opacity
    } else if pointer_in_viewport {
        scroll_style.active_handle_opacity
    } else {
        scroll_style.dormant_handle_opacity
    };

    // Animate opacity for smooth transitions
    let opacity_id = ui.id().with("scrollbar_opacity");
    let dt = ui.input(|i| i.predicted_dt).min(0.05);
    let opacity = ui.ctx().data_mut(|d| {
        let current = d.get_temp_mut_or_insert_with::<f32>(opacity_id, || 0.0_f32);
        let speed = if handle_opacity > *current { 12.0 } else { 6.0 };
        *current += (handle_opacity - *current) * (dt * speed).min(1.0);
        if (*current - handle_opacity).abs() < 0.01 {
            *current = handle_opacity;
        }
        *current
    });

    if opacity > 0.005 {
        let base_color = ui.visuals().widgets.inactive.fg_stroke.color;
        let color = Color32::from_rgba_unmultiplied(
            base_color.r(),
            base_color.g(),
            base_color.b(),
            (opacity * 255.0) as u8,
        );
        ui.painter().rect_filled(handle_rect, bar_w / 2.0, color);
    }

    if (opacity - handle_opacity).abs() > 0.01 {
        ui.ctx().request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::scroll_interpolation_factor;

    #[test]
    fn opengl_interpolation_preserves_60_hz_response() {
        assert!((scroll_interpolation_factor(1.0 / 60.0, true) - 0.15).abs() < 0.000_01);
    }

    #[test]
    fn opengl_interpolation_depends_on_elapsed_time_not_frame_count() {
        let one_frame_remaining = 1.0 - scroll_interpolation_factor(1.0 / 30.0, true);
        let half_frame_remaining = 1.0 - scroll_interpolation_factor(1.0 / 60.0, true);
        assert!((one_frame_remaining - half_frame_remaining.powi(2)).abs() < 0.000_01);
    }

    #[test]
    fn non_opengl_interpolation_keeps_existing_linear_step() {
        assert!((scroll_interpolation_factor(1.0 / 60.0, false) - 0.15).abs() < f32::EPSILON);
    }
}
