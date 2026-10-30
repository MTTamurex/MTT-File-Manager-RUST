use super::*;

#[test]
fn covering_window_blocks_geometric_hover_and_click_hit_testing() {
    let ctx = egui::Context::default();
    let mut covered = egui::Pos2::ZERO;
    for frame in 0..3 {
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            let response = egui::Window::new("Comparison")
                .fixed_pos(egui::pos2(100.0, 100.0))
                .fixed_size(egui::vec2(300.0, 180.0))
                .show(ui.ctx(), |ui| {
                    ui.label("Comparison content");
                })
                .unwrap();
            covered = response.response.rect.center();
            if frame > 0 {
                assert!(!allows_pointer(ui, covered));
                assert!(allows_pointer(ui, egui::pos2(10.0, 10.0)));
            }
        });
    }
}

#[test]
fn own_tooltip_does_not_disable_the_treemap() {
    let ctx = egui::Context::default();
    for frame in 0..3 {
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            let response = egui::Area::new(tooltip_id())
                .order(egui::Order::Tooltip)
                .interactable(false)
                .fixed_pos(egui::pos2(100.0, 100.0))
                .show(ui.ctx(), |ui| {
                    ui.label("Tile details");
                });
            if frame > 0 {
                assert!(allows_pointer(ui, response.response.rect.center()));
            }
        });
    }
}
