use super::*;

#[test]
fn comparison_is_centered_on_open_and_reopen_without_pinning_its_position() {
    let ctx = egui::Context::default();
    center_on_open(&ctx);
    for _ in 0..3 {
        let _ = ctx.run_ui(Default::default(), |ui| {
            let window = comparison_window(ui.ctx())
                .show(ui.ctx(), |ui| {
                    ui.label("Example copy");
                })
                .unwrap();
            assert!(
                window
                    .response
                    .rect
                    .center()
                    .distance(ctx.viewport_rect().center())
                    < 2.0
            );
            assert!(window.inner.is_some());
        });
    }
    let moved = ctx.viewport_rect().center() + egui::vec2(80.0, 60.0);
    let _ = ctx.run_ui(Default::default(), |ui| {
        comparison_window(ui.ctx())
            .movable(false)
            .current_pos(moved)
            .show(ui.ctx(), |ui| {
                ui.label("Example copy");
            });
    });
    let _ = ctx.run_ui(Default::default(), |ui| {
        let window = comparison_window(ui.ctx())
            .show(ui.ctx(), |ui| {
                ui.label("Example copy");
            })
            .unwrap();
        assert!(window.response.rect.center().distance(moved) < 2.0);
    });
    for _ in 0..2 {
        let _ = ctx.run_ui(Default::default(), |_| {});
    }
    center_on_open(&ctx);
    let _ = ctx.run_ui(Default::default(), |ui| {
        let window = comparison_window(ui.ctx())
            .show(ui.ctx(), |ui| {
                ui.label("Example copy");
            })
            .unwrap();
        assert!(
            window
                .response
                .rect
                .center()
                .distance(ctx.viewport_rect().center())
                < 2.0
        );
    });
}
