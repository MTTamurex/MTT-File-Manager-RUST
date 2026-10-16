use eframe::egui;

const EDGE_MARGIN: f32 = 4.0;
const FRAME_MARGIN: f32 = 8.0;

fn scroll_ids_key() -> egui::Id {
    egui::Id::new("context_menu_scroll_ids")
}

pub(super) fn reset_scroll(ctx: &egui::Context) {
    let ids = ctx.data_mut(|data| {
        let ids = data
            .get_temp::<Vec<egui::Id>>(scroll_ids_key())
            .unwrap_or_default();
        data.remove::<Vec<egui::Id>>(scroll_ids_key());
        ids
    });
    for id in ids {
        egui::scroll_area::State::default().store(ctx, id);
    }
}

pub(super) fn content_height(viewport: egui::Rect) -> f32 {
    (viewport.height() - 2.0 * EDGE_MARGIN - FRAME_MARGIN).max(0.0)
}

pub(super) fn clamp_position(
    viewport: egui::Rect,
    position: egui::Pos2,
    expected_height: f32,
) -> egui::Pos2 {
    let top = viewport.top() + EDGE_MARGIN;
    let height = expected_height.min(content_height(viewport) + FRAME_MARGIN);
    let bottom = (viewport.bottom() - EDGE_MARGIN - height).max(top);
    egui::pos2(position.x, position.y.clamp(top, bottom))
}

pub(super) fn scroll<R>(
    ui: &mut egui::Ui,
    id: egui::Id,
    viewport: egui::Rect,
    contents: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::scroll_area::ScrollAreaOutput<R> {
    let output = egui::ScrollArea::vertical()
        .id_salt(id.with("scroll"))
        .max_height(content_height(viewport))
        .auto_shrink([false, true])
        .show(ui, contents);
    ui.ctx().data_mut(|data| {
        let ids = data.get_temp_mut_or_default::<Vec<egui::Id>>(scroll_ids_key());
        if !ids.contains(&output.id) {
            ids.push(output.id);
        }
    });
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_sessions_reset_all_scroll_offsets_but_lazy_updates_preserve_them() {
        let ctx = egui::Context::default();
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 300.0));
        let input = egui::RawInput {
            screen_rect: Some(viewport),
            ..Default::default()
        };
        let mut menu = super::super::ContextMenuState::default();
        menu.open(egui::Pos2::ZERO, None, Vec::new(), true);
        assert!(super::super::reset_submenu_hierarchy_for_new_session(
            &mut menu
        ));
        reset_scroll(&ctx);
        for frame in 0..4 {
            if frame == 3 {
                menu.open(egui::Pos2::ZERO, None, Vec::new(), true);
                assert!(super::super::reset_submenu_hierarchy_for_new_session(
                    &mut menu
                ));
                reset_scroll(&ctx);
            } else {
                assert!(!super::super::reset_submenu_hierarchy_for_new_session(
                    &mut menu
                ));
            }
            let _ = ctx.run_ui(input.clone(), |ui| {
                ui.columns(2, |columns| {
                    for (index, ui) in columns.iter_mut().enumerate() {
                        let mut output =
                            scroll(ui, egui::Id::new(("menu", index)), viewport, |ui| {
                                // A lazy update adds items within the same menu session.
                                for _ in 0..(50 + frame) {
                                    ui.allocate_exact_size(
                                        egui::vec2(100.0, 30.0),
                                        egui::Sense::hover(),
                                    );
                                }
                            });
                        if frame == 0 {
                            output.state.offset.y = 150.0;
                            output.state.store(&ctx, output.id);
                        } else if frame == 3 {
                            assert_eq!(output.state.offset.y, 0.0);
                        } else {
                            assert_eq!(output.state.offset.y, 150.0);
                        }
                    }
                });
            });
        }
    }

    #[test]
    fn tall_popup_fits_offset_viewport_and_all_rows_are_scrollable() {
        let ctx = egui::Context::default();
        let viewport = egui::Rect::from_min_size(egui::pos2(20.0, 30.0), egui::vec2(400.0, 240.0));
        let position = clamp_position(viewport, egui::pos2(80.0, 250.0), 1000.0);
        assert_eq!(position.y, viewport.top() + EDGE_MARGIN);
        let input = egui::RawInput {
            screen_rect: Some(viewport),
            ..Default::default()
        };
        let mut offset = 0.0;
        // Establish the viewport first; scroll-to requests use previous geometry.
        for _ in 0..3 {
            let _ = ctx.run_ui(input.clone(), |ui| {
                let output = scroll(ui, egui::Id::new("tall_menu"), viewport, |ui| {
                    for _ in 0..50 {
                        ui.allocate_exact_size(egui::vec2(200.0, 30.0), egui::Sense::hover());
                    }
                    ui.scroll_to_cursor_animation(
                        Some(egui::Align::BOTTOM),
                        egui::style::ScrollAnimation::none(),
                    );
                });
                assert!(output.inner_rect.height() <= content_height(viewport) + 0.1);
                assert!(output.content_size.y > output.inner_rect.height());
                offset = output.state.offset.y;
            });
        }
        assert!(offset > 0.0);
    }
}
