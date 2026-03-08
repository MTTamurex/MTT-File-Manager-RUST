use eframe::egui::{self, Color32, Rect, Sense, Ui};

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
    if consume_scroll {
        let scroll_delta = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll_delta != 0.0 {
            let speed = 2.5;
            *target_scroll -= scroll_delta * speed;
        }
    }

    *target_scroll = target_scroll.clamp(0.0, max_scroll);
}

pub(super) fn compute_visual_scroll(ui: &Ui, target_scroll: f32, viewport_h: f32) -> (f32, f32) {
    let scroll_state_id = ui.id().with("scroll_state");
    // Use predicted_dt (fixo, ~16.67ms) em vez de stable_dt (variável).
    // stable_dt herda picos de latência do eframe/wgpu (tessellation+present),
    // causando o lerp a "pular" nos frames lentos e "voltar" nos seguintes.
    // predicted_dt é constante e garante movimento visual uniforme.
    let dt = ui.input(|i| i.predicted_dt).min(0.05);

    let visual_scroll = ui.ctx().data_mut(|d| {
        let state = d.get_temp_mut_or_insert_with::<ScrollState>(scroll_state_id, || ScrollState {
            visual_scroll_y: target_scroll,
        });

        let t = (dt * 25.0).min(1.0);

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
        // FIX: request_repaint() (imediato) em vez de request_repaint_after(16ms).
        //
        // O timer de 16ms não é sincronizado com o vsync do wgpu/driver. Quando
        // o timer dispara no meio de um ciclo de vsync (ex: 3ms antes do próximo
        // present), o eframe agenda o update() tarde demais → perde a janela do
        // vsync → o present() espera mais um ciclo (~16.7ms extra) → dt efetivo
        // de ~33-42ms em vez de ~16.7ms. Isso acontece a cada ~1 segundo,
        // criando o padrão rítmico de micro stutter observado nos logs.
        //
        // request_repaint() pede "o mais cedo possível", e o eframe sincroniza
        // naturalmente com o vsync, eliminando o conflito de timing.
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

    let scrollbar_w = 12.0;
    let scrollbar_rect = Rect::from_min_max(
        viewport_rect.right_top() - egui::vec2(scrollbar_w, 0.0),
        viewport_rect.right_bottom(),
    );

    ui.painter()
        .rect_filled(scrollbar_rect, 0.0, Color32::from_gray(245));

    let handle_h = (viewport_h / total_content_height * viewport_h)
        .max(30.0)
        .min(viewport_h.max(30.0));
    let travel = (viewport_h - handle_h).max(1.0);
    let handle_y = (current_scroll / max_scroll) * travel;
    let handle_rect = Rect::from_min_size(
        scrollbar_rect.min + egui::vec2(2.0, handle_y),
        egui::vec2(scrollbar_w - 4.0, handle_h),
    );

    let interact = ui.interact(
        scrollbar_rect,
        ui.id().with("scrollbar"),
        Sense::click_and_drag(),
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

    let color = if interact.dragged() {
        Color32::from_gray(150)
    } else if interact.hovered() {
        Color32::from_gray(180)
    } else {
        Color32::from_gray(200)
    };
    ui.painter().rect_filled(handle_rect, 4.0, color);
}
