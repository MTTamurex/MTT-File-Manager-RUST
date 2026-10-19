use eframe::egui::{self, Rect, Sense};

pub(in crate::ui::app::panels) struct TagsBlockLayout {
    pub height: f32,
    pub min_height: f32,
    pub max_height: f32,
    pub collapsed: bool,
    pub drives_height: f32,
    pub drives_min_height: f32,
    pub drives_max_height: f32,
}

pub(in crate::ui::app::panels) struct TagsBlockOutput<R> {
    pub inner: R,
    pub interaction_rect: Rect,
    pub resized_height: Option<f32>,
    pub reset_height: bool,
    pub resized_drives_height: Option<f32>,
    pub reset_drives_height: bool,
    pub height_changed: bool,
}

/// The upper divider always resizes drives; the lower one always resizes Tags.
pub(in crate::ui::app::panels) fn render_tags_block<R>(
    ui: &mut egui::Ui,
    layout: TagsBlockLayout,
    render: impl FnOnce(&mut egui::Ui) -> R,
) -> TagsBlockOutput<R> {
    let top_divider = divider(ui, egui::Id::new("sidebar_drives_divider_drag"), true);
    // Tags cannot grow past the visible space below the drives and two dividers.
    let tags_max_height = layout
        .max_height
        .min((ui.available_height() - 9.0 - ui.spacing().item_spacing.y).max(0.0));
    let scroll_scope = ui.scope(|ui| {
        ui.spacing_mut().scroll.fade.strength = 0.0;
        egui::ScrollArea::vertical()
            .id_salt("sidebar_tags_scroll")
            .auto_shrink([false, true])
            .max_height(layout.height.min(tags_max_height))
            .min_scrolled_height(0.0)
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                render(ui)
            })
    });
    let tags_rect = scroll_scope.response.rect;
    let output = scroll_scope.inner;
    let bottom_divider = divider(
        ui,
        egui::Id::new("sidebar_tags_divider_drag"),
        !layout.collapsed,
    );
    let resized_height = bottom_divider.dragged().then(|| {
        ui.ctx().request_repaint();
        (tags_rect.height() + bottom_divider.drag_delta().y)
            .clamp(layout.min_height.min(tags_max_height), tags_max_height)
    });
    let resized_drives_height = top_divider.dragged().then(|| {
        ui.ctx().request_repaint();
        (layout.drives_height + top_divider.drag_delta().y)
            .clamp(layout.drives_min_height, layout.drives_max_height)
    });
    let interaction_rect = tags_rect.union(top_divider.rect).union(bottom_divider.rect);
    TagsBlockOutput {
        inner: output.inner,
        interaction_rect,
        resized_height,
        reset_height: bottom_divider.double_clicked(),
        resized_drives_height,
        reset_drives_height: top_divider.double_clicked(),
        height_changed: top_divider.drag_stopped()
            || bottom_divider.drag_stopped()
            || top_divider.double_clicked()
            || bottom_divider.double_clicked(),
    }
}

fn divider(ui: &mut egui::Ui, id: egui::Id, enabled: bool) -> egui::Response {
    let (mut rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 9.0), Sense::hover());
    rect.min.x = ui.clip_rect().min.x;
    rect.max.x = ui.clip_rect().max.x;
    let response = ui.interact(
        rect,
        if enabled { id } else { id.with("separator") },
        if enabled {
            Sense::click_and_drag()
        } else {
            Sense::hover()
        },
    );
    if enabled && (response.hovered() || response.dragged()) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }
    if ui.is_rect_visible(rect) {
        let stroke = if enabled && (response.hovered() || response.dragged()) {
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
                for at_bottom in [false, true] {
                    for delta in [-30.0, 30.0] {
                        let ctx = egui::Context::default();
                        let mut height = 160.0;
                        let mut drives_limit = Some(160.0);
                        let mut frame = |events| {
                            let mut handle = Rect::NOTHING;
                            let mut resized = None;
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
                                        ui.allocate_exact_size(
                                            egui::vec2(240.0, 48.0),
                                            Sense::hover(),
                                        );
                                    }
                                    let drives_available = (ui.available_height()
                                        - height
                                        - super::super::TAGS_DIVIDER_HEIGHT
                                        - 4.0 * ui.spacing().item_spacing.y)
                                        .max(0.0);
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
                                            height,
                                            min_height: 30.0,
                                            max_height: 350.0,
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
                                    resized = output.resized_height;
                                    resized_drives = output.resized_drives_height;
                                    if resized.is_some() {
                                        drives_limit = Some(drives_rendered_height);
                                    }
                                },
                            );
                            if let Some(resized) = resized {
                                height = resized;
                            }
                            if let Some(resized) = resized_drives {
                                drives_limit = Some(resized);
                            }
                            egui::pos2(
                                handle.center().x,
                                if at_bottom {
                                    handle.bottom() - 4.5
                                } else {
                                    handle.top() + 4.5
                                },
                            )
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
                        let expected_delta = if !at_bottom && drives_height < 160.0 {
                            delta.min(0.0)
                        } else {
                            delta
                        };
                        assert!((moved.y - origin.y - expected_delta).abs() < 1.0, "handle must follow pointer up to content bounds for drives_height={drives_height}, delta={delta}: {moved:?} vs {destination:?}");
                        frame(vec![egui::Event::PointerButton {
                            pos: destination,
                            button: egui::PointerButton::Primary,
                            pressed: false,
                            modifiers: egui::Modifiers::NONE,
                        }]);
                        if at_bottom {
                            assert!((height - (160.0 + delta)).abs() < 1.0);
                        } else {
                            let expected = (drives_height.min(160.0) + delta).min(drives_height);
                            assert!((drives_limit.unwrap() - expected).abs() < 1.0);
                            assert_eq!(height, 160.0);
                        }
                    }
                }
            }
        }
    }
}
