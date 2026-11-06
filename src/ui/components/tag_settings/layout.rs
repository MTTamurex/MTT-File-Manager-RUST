use eframe::egui;

#[derive(Clone, Copy)]
pub(super) enum Pane {
    List,
    Details,
}

pub(super) fn show(ui: &mut egui::Ui, mut contents: impl FnMut(&mut egui::Ui, Pane, f32)) {
    // Bound both panes to the viewport, even when hosted in a ScrollArea.
    // The toolbar remains outside both inner scrolls.
    let height = ui
        .available_height()
        .min(ui.clip_rect().bottom() - ui.cursor().top() - 8.0)
        .clamp(0.0, 640.0);
    if height <= 0.0 {
        return;
    }
    if ui.available_width() >= 600.0 {
        let list_width = (ui.available_width() * 0.43).clamp(220.0, 380.0);
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(list_width, height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(list_width);
                    contents(ui, Pane::List, height);
                },
            );
            // A native vertical separator uses all available height, which
            // can be infinite when this browser is hosted in a ScrollArea.
            let (separator, _) =
                ui.allocate_exact_size(egui::vec2(1.0, height), egui::Sense::hover());
            ui.painter().vline(
                separator.center().x,
                separator.y_range(),
                ui.visuals().widgets.noninteractive.bg_stroke,
            );
            let width = ui.available_width();
            ui.allocate_ui_with_layout(
                egui::vec2(width, height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(width);
                    contents(ui, Pane::Details, height);
                },
            );
        });
    } else {
        let separator_height = 8.0;
        let content_height =
            (height - separator_height - ui.spacing().item_spacing.y * 2.0).max(0.0);
        if content_height <= 0.0 {
            return;
        }
        let list_height = content_height * 0.42;
        contents(ui, Pane::List, list_height);
        ui.add(egui::Separator::default().spacing(separator_height));
        contents(ui, Pane::Details, content_height - list_height);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panes_never_exceed_the_remaining_viewport() {
        for height in [160.0, 200.0, 240.0] {
            let ctx = egui::Context::default();
            let _ = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(360.0, height),
                    )),
                    ..Default::default()
                },
                |ui| {
                    ui.add_space(120.0);
                    let bottom = ui.clip_rect().bottom();
                    show(ui, |ui, pane, height| {
                        let output = egui::ScrollArea::vertical()
                            .id_salt(matches!(pane, Pane::List))
                            .max_height(height)
                            .min_scrolled_height(0.0)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.label("Item");
                            });
                        assert!(
                            output.inner_rect.bottom() <= bottom + 1.0,
                            "minimum pane heights must not push details below the viewport"
                        );
                    });
                },
            );
        }
    }

    #[test]
    fn short_viewports_keep_panes_below_the_fixed_toolbar() {
        for width in [360.0, 800.0] {
            let ctx = egui::Context::default();
            let _ = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 300.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    ui.label("Section header");
                    ui.add_space(100.0);
                    let toolbar = ui.text_edit_singleline(&mut String::new()).rect;
                    let bottom = ui.clip_rect().bottom();
                    show(ui, |ui, pane, height| {
                        let output = egui::ScrollArea::vertical()
                            .id_salt(matches!(pane, Pane::List))
                            .max_height(height)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                for _ in 0..100 {
                                    ui.label("Item");
                                }
                            });
                        assert!(output.inner_rect.top() >= toolbar.bottom());
                        assert!(output.inner_rect.bottom() <= bottom + 1.0);
                    });
                },
            );
        }
    }

    #[test]
    fn panes_fit_narrow_and_wide_windows_with_independent_bounded_scrolls() {
        for width in [280.0, 480.0, 640.0, 1000.0] {
            let ctx = egui::Context::default();
            for frame in 0..3 {
                let _ = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 500.0),
                        )),
                        time: Some(frame as f64),
                        ..Default::default()
                    },
                    |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("outer")
                            .show(ui, |ui| {
                                ui.label("Toolbar");
                                let max_width = ui.available_width();
                                let mut rects = Vec::new();
                                show(ui, |ui, pane, height| {
                                    let output = egui::ScrollArea::vertical()
                                        .id_salt(matches!(pane, Pane::List))
                                        .max_height(height)
                                        .auto_shrink([false, false])
                                        .show(ui, |ui| {
                                            for _ in 0..100 {
                                                ui.label("Generic item");
                                            }
                                        });
                                    assert!(output.inner_rect.height() <= height + 1.0);
                                    rects.push(output.inner_rect);
                                });
                                assert_eq!(rects.len(), 2);
                                assert!(ui.min_rect().width() <= max_width + 1.0);
                                assert!(ui.min_rect().bottom() <= ui.clip_rect().bottom() + 1.0,
                                    "a vertical separator must not expand the scroll content height");
                                if max_width >= 600.0 {
                                    assert!(rects[0].right() <= rects[1].left());
                                } else {
                                    assert!(rects[0].bottom() <= rects[1].top());
                                }
                            });
                    },
                );
            }
        }
    }
}
