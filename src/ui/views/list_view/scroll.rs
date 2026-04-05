use eframe::egui::Ui;

const LIST_SCROLL_SPEED: f32 = 5.0;

#[derive(Clone, Copy, Debug)]
struct ScrollState {
    visual_scroll_y: f32,
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
        *target_scroll += -scroll_delta * LIST_SCROLL_SPEED;
    }

    *target_scroll = target_scroll.clamp(0.0, max_scroll);
}

pub(super) fn compute_visual_scroll(
    ui: &Ui,
    target_scroll: f32,
    viewport_h: f32,
    generation: usize,
) -> (f32, f32) {
    let scroll_state_id = ui.id().with("list_scroll_state").with(generation);
    let dt = ui.input(|i| i.predicted_dt).min(0.05);

    let visual_scroll = ui.ctx().data_mut(|d| {
        let state = d.get_temp_mut_or_insert_with::<ScrollState>(scroll_state_id, || ScrollState {
            visual_scroll_y: target_scroll,
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