use eframe::egui::{self, Rect, Sense};

pub(in crate::ui::app::panels) struct TagsBlockLayout {
    pub collapsed: bool,
    pub drives_height: f32,
    pub drives_min_height: f32,
    pub drives_max_height: f32,
}

pub(in crate::ui::app::panels) struct TagsBlockOutput<R> {
    pub inner: R,
    pub interaction_rect: Rect,
    pub resized_drives_height: Option<f32>,
    pub reset_drives_height: bool,
    pub height_changed: bool,
}

/// The divider resizes drives; Tags fills the remaining sidebar height.
pub(in crate::ui::app::panels) fn render_tags_block<R>(
    ui: &mut egui::Ui,
    layout: TagsBlockLayout,
    render: impl FnOnce(&mut egui::Ui) -> R,
) -> TagsBlockOutput<R> {
    let top_divider = divider(ui, egui::Id::new("sidebar_drives_divider_drag"));
    let scroll_scope = ui.scope(|ui| {
        ui.spacing_mut().scroll.fade.strength = 0.0;
        egui::ScrollArea::vertical()
            .id_salt("sidebar_tags_scroll")
            .auto_shrink([false, layout.collapsed])
            .max_height(ui.available_height().max(0.0))
            .min_scrolled_height(0.0)
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                render(ui)
            })
    });
    let tags_rect = scroll_scope.response.rect;
    let output = scroll_scope.inner;
    let resized_drives_height = top_divider.dragged().then(|| {
        ui.ctx().request_repaint();
        (layout.drives_height + top_divider.drag_delta().y)
            .clamp(layout.drives_min_height, layout.drives_max_height)
    });
    let interaction_rect = tags_rect.union(top_divider.rect);
    TagsBlockOutput {
        inner: output.inner,
        interaction_rect,
        resized_drives_height,
        reset_drives_height: top_divider.double_clicked(),
        height_changed: top_divider.drag_stopped() || top_divider.double_clicked(),
    }
}

fn divider(ui: &mut egui::Ui, id: egui::Id) -> egui::Response {
    let (mut rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 9.0), Sense::hover());
    rect.min.x = ui.clip_rect().min.x;
    rect.max.x = ui.clip_rect().max.x;
    let response = ui.interact(rect, id, Sense::click_and_drag());
    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }
    if ui.is_rect_visible(rect) {
        let stroke = if response.hovered() || response.dragged() {
            egui::Stroke::new(1.5, ui.visuals().widgets.active.bg_fill)
        } else {
            ui.visuals().widgets.noninteractive.bg_stroke
        };
        ui.painter().hline(rect.x_range(), rect.center().y, stroke);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidebar_dividers_track_drag_with_and_without_quick_access() {
        for quick_access in [false, true] {
            for drives_height in [120.0_f32, 200.0, 800.0] {
                for delta in [-30.0, 30.0, 200.0] {
                    let ctx = egui::Context::default();
                    let mut drives_limit = Some(160.0);
                    let mut frame = |events| {
                        let mut handle = Rect::NOTHING;
                        let mut resized_drives = None;
                        let _ = ctx.run_ui(
                            egui::RawInput {
                                events,
                                ..Default::default()
                            },
                            |ui| {
                                ui.set_max_size(egui::vec2(260.0, 500.0));
                                ui.set_clip_rect(ui.max_rect());
                                if quick_access {
                                    ui.allocate_exact_size(egui::vec2(240.0, 48.0), Sense::hover());
                                }
                                let drives_available = super::super::drives_available_height(
                                    ui.available_height(),
                                    true,
                                    ui.spacing().item_spacing.y,
                                );
                                let drives = egui::ScrollArea::vertical()
                                    .id_salt("test_drives")
                                    .auto_shrink([false, true])
                                    .max_height(
                                        drives_limit
                                            .unwrap_or(drives_available)
                                            .min(drives_available),
                                    )
                                    .min_scrolled_height(0.0)
                                    .show(ui, |ui| {
                                        ui.allocate_exact_size(
                                            egui::vec2(240.0, drives_height),
                                            Sense::hover(),
                                        );
                                    });
                                let drives_rendered_height =
                                    drives.content_size.y.min(drives.inner_rect.height());
                                let output = render_tags_block(
                                    ui,
                                    TagsBlockLayout {
                                        collapsed: false,
                                        drives_height: drives_rendered_height,
                                        drives_min_height: 24.0,
                                        drives_max_height: drives_available,
                                    },
                                    |ui| {
                                        ui.allocate_exact_size(
                                            egui::vec2(240.0, 600.0),
                                            Sense::hover(),
                                        );
                                    },
                                );
                                handle = output.interaction_rect;
                                resized_drives = output.resized_drives_height;
                            },
                        );
                        if let Some(resized) = resized_drives {
                            drives_limit = Some(resized);
                        }
                        egui::pos2(handle.center().x, handle.top() + 4.5)
                    };
                    let origin = frame(vec![]);
                    frame(vec![
                        egui::Event::PointerMoved(origin),
                        egui::Event::PointerButton {
                            pos: origin,
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ]);
                    let destination = origin + egui::vec2(0.0, delta);
                    frame(vec![egui::Event::PointerMoved(destination)]);
                    let moved = frame(vec![]);
                    let expected = (drives_height.min(160.0) + delta).min(drives_height);
                    let expected_delta = expected - drives_height.min(160.0);
                    assert!((moved.y - origin.y - expected_delta).abs() < 1.0, "handle must follow pointer up to content bounds for drives_height={drives_height}, delta={delta}: {moved:?} vs {destination:?}");
                    frame(vec![egui::Event::PointerButton {
                        pos: destination,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::NONE,
                    }]);
                    assert!((drives_limit.unwrap() - expected).abs() < 1.0);
                }
            }
        }
    }

    #[test]
    fn tags_fill_remaining_height_follow_collapsed_drives_and_scroll_when_needed() {
        let ctx = egui::Context::default();
        let render = |drives_height: f32, tags_height: f32, events| {
            let mut rect = Rect::NOTHING;
            let mut content_top = 0.0;
            let _ = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    ui.set_max_size(egui::vec2(260.0, 500.0));
                    ui.set_clip_rect(ui.max_rect());
                    ui.allocate_exact_size(egui::vec2(260.0, drives_height), Sense::hover());
                    let output = render_tags_block(
                        ui,
                        TagsBlockLayout {
                            collapsed: false,
                            drives_height,
                            drives_min_height: 24.0,
                            drives_max_height: 400.0,
                        },
                        |ui| {
                            let top = ui.cursor().top();
                            ui.allocate_exact_size(egui::vec2(240.0, tags_height), Sense::hover());
                            top
                        },
                    );
                    rect = output.interaction_rect;
                    content_top = output.inner;
                },
            );
            (rect, content_top)
        };
        for content_height in [50.0, 200.0, 1000.0] {
            let expanded = render(220.0, content_height, vec![]);
            let collapsed = render(60.0, content_height, vec![]);
            assert!((expanded.0.bottom() - 500.0).abs() < 0.1);
            assert!((collapsed.0.bottom() - 500.0).abs() < 0.1);
            assert!((expanded.0.top() - collapsed.0.top() - 160.0).abs() < 0.1);
            let compact = render(420.0, content_height, vec![]);
            assert!((compact.0.bottom() - 500.0).abs() < 0.1);
            assert!(compact.0.height() < collapsed.0.height());
            if content_height > compact.0.height() {
                render(
                    420.0,
                    content_height,
                    vec![
                        egui::Event::PointerMoved(compact.0.center()),
                        egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            phase: egui::TouchPhase::Move,
                            delta: egui::vec2(0.0, -60.0),
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
                let scrolled = render(420.0, content_height, vec![]);
                assert!(scrolled.1 < compact.1);
            }
        }
    }
}
