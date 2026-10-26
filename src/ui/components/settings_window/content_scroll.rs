use eframe::egui;

const CONTROL_SCROLLBAR_GAP: f32 = 8.0;

/// Reserve the scrollbar's actual width instead of letting it cover controls.
pub(super) fn show<R>(
    ui: &mut egui::Ui,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::scroll_area::ScrollAreaOutput<R> {
    ui.scope(|ui| {
        let scroll = &mut ui.spacing_mut().scroll;
        scroll.floating = false;
        scroll.bar_inner_margin = CONTROL_SCROLLBAR_GAP;
        egui::ScrollArea::vertical()
            .id_salt("settings_window_content")
            .auto_shrink([false, false])
            .show(ui, add_contents)
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::components::settings_ui;

    #[test]
    fn scrollbar_reserves_space_for_controls_at_different_widths_and_scales() {
        for width in [320.0, 640.0] {
            for pixels_per_point in [1.0, 2.0] {
                for bar_width in [10.0, 18.0] {
                    let ctx = egui::Context::default();
                    ctx.set_pixels_per_point(pixels_per_point);
                    ctx.global_style_mut(|style| {
                        style.animation_time = 0.0;
                        style.spacing.scroll.bar_width = bar_width;
                    });
                    for frame in 0..3 {
                        let _ = ctx.run_ui(
                            egui::RawInput {
                                screen_rect: Some(egui::Rect::from_min_size(
                                    egui::Pos2::ZERO, egui::vec2(width + 32.0, 240.0))),
                                time: Some(frame as f64),
                                ..Default::default()
                            },
                            |ui| {
                                ui.set_max_size(egui::vec2(width, 180.0));
                                let outer = ui.available_rect_before_wrap();
                                let original_scroll = ui.spacing().scroll;
                                let output = show(ui, |ui| {
                                    let mut enabled = true;
                                    settings_ui::toggle_row(ui, "Option", &mut enabled);
                                    let toggle_right = ui.min_rect().right();
                                    let button = ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| ui.button("Action"),
                                    ).inner.rect;
                                    // Match full-width groups used by settings pages.
                                    ui.group(|ui| {
                                        ui.set_width(ui.available_width());
                                        ui.label("Description");
                                    });
                                    ui.add_space(600.0);
                                    (toggle_right, button.right())
                                });
                                assert_eq!(ui.spacing().scroll, original_scroll);
                                if frame > 0 {
                                    let scrollbar_left = outer.right() - bar_width;
                                    for control_right in [output.inner.0, output.inner.1] {
                                        assert!(scrollbar_left - control_right >= CONTROL_SCROLLBAR_GAP - 0.1,
                                            "controls must remain outside the scrollbar's reserved lane");
                                    }
                                    assert!(output.content_size.y > output.inner_rect.height());
                                }
                            },
                        );
                    }
                }
            }
        }
    }
}
