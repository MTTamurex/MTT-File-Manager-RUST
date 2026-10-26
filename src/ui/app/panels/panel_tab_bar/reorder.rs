use crate::app::dual_panel::ActivePanel;
use eframe::egui;

pub(super) struct TabDrag {
    state_id: egui::Id,
    source: Option<usize>,
    enabled: bool,
    source_owns_drag: bool,
    tabs: Vec<(usize, egui::Rect)>,
}

impl TabDrag {
    pub fn new(ctx: &egui::Context, workspace: usize, panel: ActivePanel, item_drag: bool) -> Self {
        let state_id = egui::Id::new(("panel_tab_reorder", workspace, panel));
        let outside_release = crate::ui::tab_bar::detach::released_outside(ctx);
        let cancel = item_drag
            || ctx.input(|input| {
                input.key_pressed(egui::Key::Escape)
                    || !input.focused
                    || (!input.pointer.primary_down()
                        && !input.pointer.primary_released()
                        && !outside_release)
            });
        if cancel {
            ctx.data_mut(|data| data.remove::<usize>(state_id));
        }
        Self {
            state_id,
            source: ctx.data(|data| data.get_temp(state_id)),
            enabled: !cancel,
            source_owns_drag: false,
            tabs: Vec::new(),
        }
    }

    pub fn record_tab(
        &mut self,
        ui: &egui::Ui,
        id: usize,
        response: &egui::Response,
        close_rect: egui::Rect,
    ) -> bool {
        self.tabs.push((id, response.rect));
        let on_close = ui.input(|input| {
            input
                .pointer
                .press_origin()
                .is_some_and(|pos| close_rect.contains(pos))
        });
        let primary = egui::PointerButton::Primary;
        let started = self.enabled && response.drag_started_by(primary) && !on_close;
        if started {
            self.source = Some(id);
            ui.ctx()
                .data_mut(|data| data.insert_temp(self.state_id, id));
        }
        if self.source == Some(id) {
            self.source_owns_drag =
                response.dragged_by(primary) || response.drag_stopped_by(primary);
            if self.source_owns_drag {
                ui.painter().rect_filled(
                    response.rect,
                    egui::CornerRadius::same(7),
                    egui::Color32::from_rgba_unmultiplied(100, 120, 215, 60),
                );
            }
        }
        started
    }

    pub fn active(&self) -> bool {
        self.source.is_some()
    }

    pub fn detach_source(&self, ui: &egui::Ui) -> Option<usize> {
        let source = self.source?;
        if self.source_owns_drag && crate::ui::tab_bar::detach::released_outside(ui.ctx()) {
            ui.ctx()
                .data_mut(|data| data.remove::<usize>(self.state_id));
            self.tabs.iter().position(|(id, _)| *id == source)
        } else {
            None
        }
    }

    pub fn finish(&self, ui: &egui::Ui, visible_strip: egui::Rect) -> Option<(usize, usize)> {
        let source = self.source?;
        let from = self.tabs.iter().position(|(id, _)| *id == source);
        let (pointer, released) = ui.input(|input| {
            (
                input.pointer.interact_pos(),
                input.pointer.primary_released(),
            )
        });
        if released || from.is_none() || !self.source_owns_drag {
            ui.ctx()
                .data_mut(|data| data.remove::<usize>(self.state_id));
        }
        if !self.source_owns_drag {
            return None;
        }
        // Outside the strip there may be no more winit pointer events. Keep
        // sampling the native button until release instead of stopping here.
        if !released {
            ui.ctx().request_repaint();
        }
        let from = from?;
        let pointer = pointer?;
        let gap = drop_gap(&self.tabs, visible_strip, pointer)?;
        if released {
            return Some((from, gap));
        }
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        let x = self.tabs.get(gap).map_or_else(
            || self.tabs.last().unwrap().1.right(),
            |(_, rect)| rect.left(),
        );
        ui.painter().vline(
            x,
            self.tabs[0].1.y_range(),
            egui::Stroke::new(2.0, crate::ui::theme::accent_color(ui.ctx())),
        );
        None
    }
}

fn drop_gap(tabs: &[(usize, egui::Rect)], strip: egui::Rect, pointer: egui::Pos2) -> Option<usize> {
    let row = tabs.first()?.1.union(tabs.last()?.1).intersect(strip);
    if !row.contains(pointer) {
        return None;
    }
    Some(
        tabs.iter()
            .position(|(_, rect)| pointer.x < rect.center().x)
            .unwrap_or(tabs.len()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(ctx: &egui::Context, events: Vec<egui::Event>) -> Option<(usize, usize)> {
        frame_with_detach(ctx, events).0
    }

    fn frame_with_detach(
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> (Option<(usize, usize)>, Option<usize>) {
        let mut moved = None;
        let mut detached = None;
        let _ = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                let mut drag = TabDrag::new(ui.ctx(), 1, ActivePanel::Left, false);
                for index in 0..3 {
                    let rect = egui::Rect::from_min_size(
                        egui::pos2(100.0 + index as f32 * 100.0, 100.0),
                        egui::vec2(100.0, 28.0),
                    );
                    let response = ui.interact(
                        rect,
                        egui::Id::new(("test_tab", index)),
                        egui::Sense::click_and_drag(),
                    );
                    let close = egui::Rect::from_min_size(
                        egui::pos2(rect.right() - 20.0, rect.top()),
                        egui::vec2(20.0, 28.0),
                    );
                    drag.record_tab(ui, index, &response, close);
                }
                detached = drag.detach_source(ui);
                if detached.is_none() {
                    moved = drag.finish(
                        ui,
                        egui::Rect::from_min_max(
                            egui::pos2(100.0, 100.0),
                            egui::pos2(400.0, 128.0),
                        ),
                    );
                }
            },
        );
        (moved, detached)
    }

    fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn pointer_drag_reorders_only_on_release() {
        let ctx = egui::Context::default();
        let start = egui::pos2(120.0, 114.0);
        let end = egui::pos2(390.0, 114.0);
        frame(&ctx, vec![egui::Event::PointerMoved(start)]);
        assert_eq!(frame(&ctx, vec![button(start, true)]), None);
        assert_eq!(frame(&ctx, vec![egui::Event::PointerMoved(end)]), None);
        assert_eq!(frame(&ctx, vec![button(end, false)]), Some((0, 3)));
        assert_eq!(frame(&ctx, vec![]), None);
    }

    #[test]
    fn panel_tab_detaches_only_when_released_outside_the_window() {
        let ctx = egui::Context::default();
        let start = egui::pos2(120.0, 114.0);
        let outside = egui::pos2(850.0, 114.0);
        frame_with_detach(&ctx, vec![egui::Event::PointerMoved(start)]);
        frame_with_detach(&ctx, vec![button(start, true)]);
        assert_eq!(
            frame_with_detach(&ctx, vec![egui::Event::PointerMoved(outside)]),
            (None, None)
        );
        assert_eq!(
            frame_with_detach(&ctx, vec![button(outside, false)]),
            (None, Some(0))
        );
        assert_eq!(frame_with_detach(&ctx, vec![]), (None, None));
    }

    #[test]
    fn native_release_detaches_without_an_egui_release_event() {
        let ctx = egui::Context::default();
        let start = egui::pos2(120.0, 114.0);
        frame_with_detach(&ctx, vec![egui::Event::PointerMoved(start)]);
        frame_with_detach(&ctx, vec![button(start, true)]);
        frame_with_detach(
            &ctx,
            vec![egui::Event::PointerMoved(egui::pos2(850.0, 114.0))],
        );
        crate::ui::tab_bar::detach::set_test_pointer(&ctx, true, false);
        assert_eq!(frame_with_detach(&ctx, vec![]), (None, Some(0)));
        assert_eq!(frame_with_detach(&ctx, vec![]), (None, None));
    }

    #[test]
    fn escape_prevents_panel_detachment() {
        let ctx = egui::Context::default();
        let start = egui::pos2(120.0, 114.0);
        let outside = egui::pos2(850.0, 114.0);
        frame_with_detach(&ctx, vec![egui::Event::PointerMoved(start)]);
        frame_with_detach(&ctx, vec![button(start, true)]);
        frame_with_detach(&ctx, vec![egui::Event::PointerMoved(outside)]);
        frame_with_detach(
            &ctx,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert_eq!(
            frame_with_detach(&ctx, vec![button(outside, false)]),
            (None, None)
        );
    }

    #[test]
    fn dragging_from_close_button_does_not_reorder() {
        let ctx = egui::Context::default();
        let start = egui::pos2(190.0, 114.0);
        let end = egui::pos2(390.0, 114.0);
        frame(&ctx, vec![egui::Event::PointerMoved(start)]);
        frame(&ctx, vec![button(start, true)]);
        frame(&ctx, vec![egui::Event::PointerMoved(end)]);
        assert_eq!(frame(&ctx, vec![button(end, false)]), None);
    }

    #[test]
    fn escape_cancels_an_active_drag_before_release() {
        let ctx = egui::Context::default();
        let start = egui::pos2(120.0, 114.0);
        let end = egui::pos2(390.0, 114.0);
        frame(&ctx, vec![egui::Event::PointerMoved(start)]);
        frame(&ctx, vec![button(start, true)]);
        frame(&ctx, vec![egui::Event::PointerMoved(end)]);
        frame(
            &ctx,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert_eq!(frame(&ctx, vec![button(end, false)]), None);
    }

    #[test]
    fn stale_source_does_not_reorder_when_a_different_widget_releases() {
        let ctx = egui::Context::default();
        let start = egui::pos2(50.0, 200.0);
        let end = egui::pos2(390.0, 114.0);
        frame(&ctx, vec![egui::Event::PointerMoved(start)]);
        frame(&ctx, vec![button(start, true)]);
        frame(&ctx, vec![egui::Event::PointerMoved(end)]);
        // Simulate returning to a workspace whose drag ended while it was hidden.
        let state_id = egui::Id::new(("panel_tab_reorder", 1usize, ActivePanel::Left));
        ctx.data_mut(|data| data.insert_temp(state_id, 0usize));
        assert_eq!(frame(&ctx, vec![button(end, false)]), None);
        assert!(ctx.data(|data| data.get_temp::<usize>(state_id)).is_none());
    }

    #[test]
    fn releasing_outside_the_strip_cancels_the_drag() {
        let ctx = egui::Context::default();
        let start = egui::pos2(120.0, 114.0);
        let end = egui::pos2(450.0, 114.0);
        frame(&ctx, vec![egui::Event::PointerMoved(start)]);
        frame(&ctx, vec![button(start, true)]);
        frame(&ctx, vec![egui::Event::PointerMoved(end)]);
        assert_eq!(frame(&ctx, vec![button(end, false)]), None);
        let state_id = egui::Id::new(("panel_tab_reorder", 1usize, ActivePanel::Left));
        assert!(ctx.data(|data| data.get_temp::<usize>(state_id)).is_none());
    }

    #[test]
    fn drop_targets_include_both_ends_and_reject_the_other_panel() {
        let strip = egui::Rect::from_min_max(egui::pos2(100.0, 0.0), egui::pos2(400.0, 28.0));
        let tabs: Vec<_> = (0..3)
            .map(|id| {
                (
                    id,
                    egui::Rect::from_min_size(
                        egui::pos2(100.0 + id as f32 * 100.0, 0.0),
                        egui::vec2(100.0, 28.0),
                    ),
                )
            })
            .collect();
        assert_eq!(drop_gap(&tabs, strip, egui::pos2(110.0, 14.0)), Some(0));
        assert_eq!(drop_gap(&tabs, strip, egui::pos2(220.0, 14.0)), Some(1));
        assert_eq!(drop_gap(&tabs, strip, egui::pos2(390.0, 14.0)), Some(3));
        assert_eq!(drop_gap(&tabs, strip, egui::pos2(410.0, 14.0)), None);
        assert_eq!(drop_gap(&tabs, strip, egui::pos2(200.0, 40.0)), None);
        assert_eq!(drop_gap(&[], strip, strip.center()), None);
    }

    #[test]
    fn drag_state_is_scoped_to_workspace_and_panel_and_cancelled_by_item_drag() {
        let ctx = egui::Context::default();
        let first = TabDrag::new(&ctx, 1, ActivePanel::Left, false);
        ctx.data_mut(|data| data.insert_temp(first.state_id, 7usize));
        assert_ne!(
            first.state_id,
            TabDrag::new(&ctx, 1, ActivePanel::Right, false).state_id
        );
        assert_ne!(
            first.state_id,
            TabDrag::new(&ctx, 2, ActivePanel::Left, false).state_id
        );
        assert!(!TabDrag::new(&ctx, 1, ActivePanel::Left, true).active());
        assert!(ctx
            .data(|data| data.get_temp::<usize>(first.state_id))
            .is_none());
    }
}
