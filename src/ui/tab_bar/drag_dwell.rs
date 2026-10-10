use eframe::egui;

pub(crate) fn should_activate_tab_on_drag_hover(
    ui: &mut egui::Ui,
    dwell_id: egui::Id,
    is_item_dragging: bool,
    is_active: bool,
    pointer_over: bool,
) -> bool {
    if !is_item_dragging || is_active || !pointer_over {
        ui.ctx().data_mut(|d| d.remove::<f64>(dwell_id));
        return false;
    }

    let now = ui.input(|i| i.time);
    let dwell_start = ui
        .ctx()
        .data_mut(|d| *d.get_temp_mut_or_insert_with(dwell_id, || now));
    let elapsed = (now - dwell_start) as f32;
    if elapsed >= 0.4 {
        ui.ctx().data_mut(|d| d.remove::<f64>(dwell_id));
        return true;
    }
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_secs_f32(0.4 - elapsed + 0.02));
    false
}

#[cfg(test)]
mod tests {
    use super::should_activate_tab_on_drag_hover;
    use eframe::egui;

    fn tick(
        ctx: &egui::Context,
        dwell_id: egui::Id,
        time: f64,
        is_item_dragging: bool,
        is_active: bool,
        pointer_over: bool,
    ) -> bool {
        let mut activated = false;
        let _ = ctx.run_ui(
            egui::RawInput {
                time: Some(time),
                ..Default::default()
            },
            |ui| {
                activated = should_activate_tab_on_drag_hover(
                    ui,
                    dwell_id,
                    is_item_dragging,
                    is_active,
                    pointer_over,
                );
            },
        );
        activated
    }

    #[test]
    fn activates_a_different_tab_after_hover_dwell() {
        let ctx = egui::Context::default();
        let tab_id = egui::Id::new("target_panel_tab");

        assert!(!tick(&ctx, tab_id, 1.0, true, false, true));
        assert!(!tick(&ctx, tab_id, 1.3, true, false, true));
        assert!(tick(&ctx, tab_id, 1.41, true, false, true));
    }

    #[test]
    fn dwell_timers_are_independent_and_do_not_activate_the_current_tab() {
        let ctx = egui::Context::default();
        let first_tab = egui::Id::new("first_panel_tab");
        let second_tab = egui::Id::new("second_panel_tab");

        assert!(!tick(&ctx, first_tab, 1.0, true, false, true));
        assert!(!tick(&ctx, second_tab, 1.2, true, false, true));
        assert!(!tick(&ctx, first_tab, 1.41, true, true, true));
        assert!(tick(&ctx, second_tab, 1.61, true, false, true));
    }

    #[test]
    fn ending_a_drag_clears_an_incomplete_dwell_timer() {
        let ctx = egui::Context::default();
        let tab_id = egui::Id::new("target_panel_tab");

        assert!(!tick(&ctx, tab_id, 1.0, true, false, true));
        assert!(!tick(&ctx, tab_id, 1.5, false, false, true));
        assert!(!tick(&ctx, tab_id, 1.6, true, false, true));
        assert!(tick(&ctx, tab_id, 2.01, true, false, true));
    }
}
