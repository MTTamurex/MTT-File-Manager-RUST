use super::{SidebarAction, SidebarContext};
use eframe::egui::{self, Color32, Pos2, Rect, Sense};
use rust_i18n::t;

const TAG_HEADER_H: f32 = 22.0;
const TAG_ROW_H: f32 = 26.0;
const TAG_BOTTOM_PADDING_H: f32 = 8.0;
const AUTO_SCROLL_MARGIN: f32 = 30.0;
const AUTO_SCROLL_SPEED: f32 = 8.0;

/// Renders the Tags section inside its independent sidebar `ScrollArea`.
pub fn render_tags_section(
    ui: &mut egui::Ui,
    ctx: &mut SidebarContext,
    action: &mut Option<SidebarAction>,
) {
    if !ctx.show_tags || ctx.tag_definitions.is_empty() {
        return;
    }

    let drag_id = egui::Id::new("sidebar_tag_reorder_drag");
    let drag_source: Option<i64> = ui.ctx().data(|data| data.get_temp(drag_id));
    let pointer_pos = ui.ctx().input(|input| {
        input
            .pointer
            .hover_pos()
            .or_else(|| input.pointer.interact_pos())
    });
    let (primary_released, cancel_drag) = ui.ctx().input(|input| {
        let primary_released = input.pointer.primary_released();
        let cancel_drag = input.key_pressed(egui::Key::Escape)
            || !input.viewport().focused.unwrap_or(true)
            || (!input.pointer.primary_down() && !primary_released);
        (primary_released, cancel_drag)
    });
    let clip_rect = ui.clip_rect();

    let (header_rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), TAG_HEADER_H),
        Sense::hover(),
    );
    let toggle_size = 18.0;
    let toggle_rect = Rect::from_center_size(
        Pos2::new(
            header_rect.max.x - toggle_size / 2.0 - 3.0,
            header_rect.center().y,
        ),
        egui::vec2(toggle_size, toggle_size),
    );
    let toggle_response = ui
        .interact(
            toggle_rect,
            egui::Id::new("sidebar_toggle_tags"),
            Sense::click(),
        )
        .on_hover_text(if ctx.collapse_tags {
            t!("sidebar.expand_section")
        } else {
            t!("sidebar.collapse_section")
        });

    if ui.is_rect_visible(header_rect) {
        ui.painter().text(
            Pos2::new(header_rect.min.x + 8.0, header_rect.center().y),
            egui::Align2::LEFT_CENTER,
            t!("tags.section"),
            egui::FontId::proportional(10.0),
            Color32::from_gray(120),
        );

        let toggle_icon = if ctx.collapse_tags { "v" } else { "^" };
        let toggle_color = if toggle_response.hovered() {
            ui.visuals().text_color()
        } else {
            Color32::from_gray(140)
        };
        ui.painter().text(
            toggle_rect.center(),
            egui::Align2::CENTER_CENTER,
            toggle_icon,
            egui::FontId::proportional(14.0),
            toggle_color,
        );
    }

    if toggle_response.clicked() && action.is_none() {
        *action = Some(SidebarAction::ToggleTags);
    }

    if ctx.collapse_tags {
        ui.add_space(TAG_BOTTOM_PADDING_H);
        return;
    }

    ui.add_space(4.0);

    let ordered_ids: Vec<i64> = ctx
        .sidebar_tag_ids
        .iter()
        .copied()
        .filter(|id| ctx.tag_definitions.contains_key(id))
        .collect();
    let mut drop_target: Option<Option<i64>> = None;

    for (index, tag_id) in ordered_ids.iter().copied().enumerate() {
        let Some(tag) = ctx.tag_definitions.get(&tag_id) else {
            continue;
        };
        let is_selected = ctx.active_tag_filter == Some(tag.id);
        let count = ctx.tag_counts.get(&tag.id).copied().unwrap_or(0);
        let (mut rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), TAG_ROW_H),
            Sense::click_and_drag(),
        );
        rect.min.x = clip_rect.min.x;
        rect.max.x = clip_rect.max.x;

        // Start internal drag for reordering (anywhere on the row, like Quick Access)
        if response.drag_started() {
            ui.ctx().data_mut(|data| data.insert_temp(drag_id, tag.id));
        }

        let mut indicator_y = None;
        if drag_source.is_some() {
            if let Some(pointer) = pointer_pos.filter(|pointer| rect.contains(*pointer)) {
                if pointer.y < rect.center().y {
                    drop_target = Some(Some(tag.id));
                    indicator_y = Some(rect.top());
                } else {
                    drop_target = Some(ordered_ids.get(index + 1).copied());
                    indicator_y = Some(rect.bottom());
                }
            }
        }

        if ui.is_rect_visible(rect) {
            let dark_mode = ui.visuals().dark_mode;
            if drag_source == Some(tag.id) {
                ui.painter().rect_filled(
                    rect,
                    0.0,
                    Color32::from_rgba_premultiplied(100, 120, 215, 60),
                );
            } else if is_selected {
                ui.painter()
                    .rect_filled(rect, 0.0, crate::ui::theme::selection_color(dark_mode));
            } else if (response.hovered() || response.dragged()) && !ctx.is_item_dragging {
                ui.painter().rect_filled(
                    rect,
                    0.0,
                    crate::ui::theme::selection_hover_color(dark_mode),
                );
            }

            let text_color = if is_selected {
                crate::ui::theme::selection_text_color(dark_mode)
            } else {
                ui.visuals().text_color()
            };
            let tag_icon_center = Pos2::new(rect.min.x + 18.0, rect.center().y);
            crate::ui::tag_icon::paint_filled(
                ui.painter(),
                tag_icon_center,
                10.0,
                tag.color.to_color32(),
            );
            ui.painter().text(
                Pos2::new(rect.min.x + 30.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                &tag.name,
                egui::FontId::proportional(11.5),
                text_color,
            );

            if count > 0 {
                ui.painter().text(
                    Pos2::new(rect.max.x - 12.0, rect.center().y),
                    egui::Align2::RIGHT_CENTER,
                    count.to_string(),
                    egui::FontId::proportional(10.0),
                    Color32::from_gray(130),
                );
            }

            if let Some(y) = indicator_y {
                ui.painter().hline(
                    rect.left()..=rect.right(),
                    y,
                    egui::Stroke::new(2.0, Color32::from_rgb(0, 120, 215)),
                );
            }
        }

        if response.clicked() && !ctx.is_renaming && action.is_none() {
            let next = if is_selected { None } else { Some(tag.id) };
            *action = Some(SidebarAction::FilterByTag(next));
        }
    }

    if let Some(source) = drag_source {
        if cancel_drag {
            ui.ctx().data_mut(|data| data.remove::<i64>(drag_id));
        } else {
            auto_scroll_during_drag(ui, pointer_pos, clip_rect);

            if primary_released {
                ui.ctx().data_mut(|data| data.remove::<i64>(drag_id));
                if let Some(before_tag_id) = drop_target {
                    if action.is_none() {
                        *action = Some(SidebarAction::ReorderTag {
                            tag_id: source,
                            before_tag_id,
                        });
                    }
                }
            }
        }
    }

    ui.add_space(TAG_BOTTOM_PADDING_H);
}

fn auto_scroll_during_drag(ui: &mut egui::Ui, pointer_pos: Option<Pos2>, viewport: Rect) {
    let Some(pointer) = pointer_pos else {
        return;
    };
    let delta = auto_scroll_delta(pointer, viewport);
    if delta == 0.0 {
        return;
    }

    let target_y = if delta < 0.0 {
        viewport.top() + delta
    } else {
        viewport.bottom() + delta
    };
    let target = Pos2::new(viewport.left(), target_y);
    let target_rect = Rect::from_min_max(target, target);
    ui.scroll_to_rect(target_rect, None);
    ui.ctx().request_repaint();
}

fn auto_scroll_delta(pointer: Pos2, viewport: Rect) -> f32 {
    if pointer.x < viewport.left() || pointer.x > viewport.right() {
        return 0.0;
    }

    if pointer.y < viewport.top() + AUTO_SCROLL_MARGIN {
        -((viewport.top() + AUTO_SCROLL_MARGIN - pointer.y).max(AUTO_SCROLL_SPEED))
    } else if pointer.y > viewport.bottom() - AUTO_SCROLL_MARGIN {
        (pointer.y - (viewport.bottom() - AUTO_SCROLL_MARGIN)).max(AUTO_SCROLL_SPEED)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::auto_scroll_delta;
    use eframe::egui::{pos2, Rect};

    #[test]
    fn auto_scrolls_near_vertical_edges_only_inside_width() {
        let viewport = Rect::from_min_max(pos2(10.0, 20.0), pos2(110.0, 220.0));

        assert!(auto_scroll_delta(pos2(50.0, 25.0), viewport) < 0.0);
        assert!(auto_scroll_delta(pos2(50.0, 215.0), viewport) > 0.0);
        assert_eq!(auto_scroll_delta(pos2(50.0, 120.0), viewport), 0.0);
        assert_eq!(auto_scroll_delta(pos2(5.0, 25.0), viewport), 0.0);
    }
}
