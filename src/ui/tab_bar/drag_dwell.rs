use eframe::egui;

pub(super) fn should_activate_tab_on_drag_hover(
    ui: &mut egui::Ui,
    idx: usize,
    is_item_dragging: bool,
    is_active: bool,
    pointer_over: bool,
) -> bool {
    if is_item_dragging && !is_active && pointer_over {
        let dwell_id = egui::Id::new("drag_tab_dwell").with(idx);
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
    } else if is_item_dragging && !pointer_over {
        let dwell_id = egui::Id::new("drag_tab_dwell").with(idx);
        ui.ctx().data_mut(|d| d.remove::<f64>(dwell_id));
    }

    false
}
