use super::folder_groups;
use super::{SidebarAction, SidebarContext};
use crate::domain::quick_access_item::{
    destination_for_quick_access_gap, ordered_quick_access_items, QuickAccessItem,
};
use eframe::egui::{self, Color32, Pos2, Rect, Sense};
use rust_i18n::t;

const QUICK_ACCESS_AUTO_SCROLL_MARGIN: f32 = 30.0;
const QUICK_ACCESS_AUTO_SCROLL_MAX_SPEED: f32 = 180.0;
const QUICK_ACCESS_AUTO_SCROLL_MAX_FRAME_DELTA: f32 = 0.05;

pub(super) fn render(
    ui: &mut egui::Ui,
    ctx: &mut SidebarContext,
    action: &mut Option<SidebarAction>,
) {
    if ctx.pinned_folders.is_empty() && ctx.folder_groups.is_empty() {
        return;
    }

    let drag_id = egui::Id::new("qa_reorder_drag");
    let drag_state: Option<(usize, f32)> = ui.ctx().data(|data| data.get_temp(drag_id));
    let drag_src = drag_state.map(|(source, _)| source);
    let viewport = ui.clip_rect();
    let pointer_pos = ui.ctx().input(|input| {
        input
            .pointer
            .hover_pos()
            .or_else(|| input.pointer.interact_pos())
    });
    let pointer_in_viewport = pointer_pos.filter(|pointer| viewport.contains(*pointer));
    let (primary_released, cancel_drag) = ui.ctx().input(|input| {
        let primary_released = input.pointer.primary_released();
        let cancel_drag = input.key_pressed(egui::Key::Escape)
            || !input.viewport().focused.unwrap_or(true)
            || (!input.pointer.primary_down() && !primary_released);
        (primary_released, cancel_drag)
    });
    let mut source_owns_drag = false;
    let ordered_items = ordered_quick_access_items(ctx.pinned_folders, ctx.folder_groups);
    let mut item_rects = Vec::with_capacity(ordered_items.len());
    let mut preview = crate::ui::tab_drag_preview::TabDragPreview::default();

    for (index, item) in ordered_items.iter().enumerate() {
        let paint_start = crate::ui::tab_drag_preview::TabDragPreview::begin(ui);
        match item {
            QuickAccessItem::PinnedFolder(path) => {
                let Some(pinned) = ctx
                    .pinned_folders
                    .iter()
                    .find(|pinned| pinned.path == *path)
                else {
                    continue;
                };
                let is_selected = !ctx.is_computer_view
                    && !ctx.is_recycle_bin_view
                    && ctx.current_path == pinned.path;
                let (mut rect, response) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), 28.0),
                    Sense::click_and_drag(),
                );
                rect.min.x = ui.clip_rect().min.x;
                rect.max.x = ui.clip_rect().max.x;

                if response.drag_started() {
                    store_drag_source(ui, rect, index, drag_id);
                }
                if drag_src == Some(index) {
                    source_owns_drag = response.dragged_by(egui::PointerButton::Primary)
                        || response.drag_stopped_by(egui::PointerButton::Primary);
                }

                let pin_size = 22.0;
                let pin_rect = Rect::from_center_size(
                    Pos2::new(rect.max.x - pin_size / 2.0 - 3.0, rect.center().y),
                    egui::vec2(pin_size, pin_size),
                );

                if should_paint_item(ui, rect, drag_src == Some(index)) {
                    if is_selected {
                        ui.painter().rect_filled(
                            rect,
                            0.0,
                            crate::ui::theme::selection_color(ui.ctx()),
                        );
                    } else if response.hovered() || response.dragged() {
                        ui.painter().rect_filled(
                            rect,
                            0.0,
                            crate::ui::theme::selection_hover_color(ui.ctx()),
                        );
                    }

                    let mut cursor_x = rect.min.x + 12.0;
                    let folder_icon = ctx
                        .icon_loader
                        .get_or_load_folder_navigation_icon(ui.ctx(), &pinned.path);
                    if let Some(icon) = folder_icon {
                        let size = icon.size_vec2();
                        let icon_rect = Rect::from_center_size(
                            Pos2::new(cursor_x + 8.0, rect.center().y),
                            size * (16.0 / size.x.max(size.y).max(1.0)),
                        );
                        ui.painter().image(
                            icon.id(),
                            icon_rect,
                            Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                    cursor_x += 24.0;

                    let text_color = if is_selected {
                        crate::ui::theme::selection_text_color(ui.ctx())
                    } else {
                        ui.visuals().text_color()
                    };
                    let display_name =
                        crate::infrastructure::onedrive::special_folder_display_name(
                            std::path::Path::new(&pinned.path),
                        )
                        .unwrap_or_else(|| pinned.display_name.clone());
                    let text_max_x = (pin_rect.left() - 8.0).min(rect.right());
                    let text_max_width = (text_max_x - cursor_x).max(0.0);
                    let font_id = egui::FontId::proportional(11.5);
                    let visible_name = crate::ui::preview_panel::utils::truncate_text_to_fit(
                        &display_name,
                        text_max_width,
                        &font_id,
                        ui,
                    );
                    ui.painter().text(
                        Pos2::new(cursor_x, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        &visible_name,
                        font_id,
                        text_color,
                    );
                    if visible_name != display_name {
                        response.clone().on_hover_text(display_name);
                    }

                    ui.interact(
                        pin_rect,
                        ui.id().with(("quick_access_pin", &pinned.path)),
                        Sense::hover(),
                    )
                    .on_hover_text(t!("sidebar.remove_from_quick_access").to_string());
                    let pin_hovered = ui
                        .input(|input| input.pointer.hover_pos())
                        .is_some_and(|position| pin_rect.contains(position));
                    let pin_color = if pin_hovered {
                        Color32::from_rgb(220, 60, 60)
                    } else {
                        Color32::from_gray(140)
                    };
                    ui.painter().text(
                        pin_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "📌",
                        egui::FontId::proportional(13.0),
                        pin_color,
                    );
                }

                preview.record(ui, rect, paint_start);
                item_rects.push(rect);

                if response.middle_clicked() && !ctx.is_renaming && action.is_none() {
                    *action = Some(SidebarAction::OpenInNewTab(pinned.path.clone()));
                }
                if response.clicked() && !ctx.is_renaming && action.is_none() {
                    let click_pos = ui.input(|input| input.pointer.interact_pos());
                    if click_pos.is_some_and(|position| pin_rect.contains(position)) {
                        *action = Some(SidebarAction::UnpinFolder(pinned.path.clone()));
                    } else {
                        *action = Some(SidebarAction::NavigateTo(pinned.path.clone()));
                    }
                }
                ui.add_space(2.0);
            }
            QuickAccessItem::FolderGroup(group_id) => {
                if let Some(group) = ctx.folder_groups.iter().find(|group| group.id == *group_id) {
                    let (rect, response) = folder_groups::render(
                        ui,
                        ctx,
                        action,
                        group,
                        index,
                        drag_id,
                        drag_src == Some(index),
                    );
                    if drag_src == Some(index) {
                        source_owns_drag = response.dragged_by(egui::PointerButton::Primary)
                            || response.drag_stopped_by(egui::PointerButton::Primary);
                    }
                    preview.record(ui, rect, paint_start);
                    item_rects.push(rect);
                }
            }
        }
    }

    let insertion_gap = vertical_drop_gap(pointer_pos, &item_rects, viewport);
    preview.paint_at_vertical_gap(
        ui,
        drag_id.with("preview"),
        if !cancel_drag && source_owns_drag {
            drag_src
        } else {
            None
        },
        viewport,
        insertion_gap,
        drag_state.map(|(_, grab_offset_y)| grab_offset_y),
    );

    if let Some(source) = drag_src {
        if cancel_drag || primary_released {
            ui.ctx()
                .data_mut(|data| data.remove::<(usize, f32)>(drag_id));
        } else if source_owns_drag {
            let delta_seconds = ui.ctx().input(|input| input.stable_dt);
            auto_scroll_during_drag(ui, pointer_pos, viewport, delta_seconds);
        }

        if primary_released && !cancel_drag && source_owns_drag && pointer_in_viewport.is_some() {
            if let Some(gap) = insertion_gap {
                let destination = destination_for_quick_access_gap(source, gap, item_rects.len());
                if destination != source && action.is_none() {
                    *action = Some(SidebarAction::ReorderQuickAccessItem {
                        from: source,
                        to: destination,
                    });
                }
            }
        }
    }
}

pub(super) fn store_drag_source(ui: &egui::Ui, rect: Rect, index: usize, drag_id: egui::Id) {
    let grab_offset_y = ui
        .input(|input| {
            input
                .pointer
                .press_origin()
                .or_else(|| input.pointer.interact_pos())
        })
        .map_or(rect.height() * 0.5, |pointer| pointer.y - rect.top());
    ui.ctx()
        .data_mut(|data| data.insert_temp(drag_id, (index, grab_offset_y)));
}

pub(super) fn should_paint_item(ui: &egui::Ui, rect: Rect, is_drag_source: bool) -> bool {
    is_drag_source || ui.is_rect_visible(rect)
}

fn vertical_drop_gap(pointer: Option<Pos2>, item_rects: &[Rect], viewport: Rect) -> Option<usize> {
    let pointer = pointer.filter(|position| viewport.contains(*position))?;
    Some(
        item_rects
            .iter()
            .position(|rect| pointer.y < rect.center().y)
            .unwrap_or(item_rects.len()),
    )
}

fn auto_scroll_during_drag(
    ui: &mut egui::Ui,
    pointer_pos: Option<Pos2>,
    viewport: Rect,
    delta_seconds: f32,
) {
    let Some(pointer) = pointer_pos else {
        return;
    };
    let delta = quick_access_auto_scroll_delta(pointer, viewport, delta_seconds);
    if delta == 0.0 {
        return;
    }

    let target_y = if delta < 0.0 {
        viewport.top() + delta
    } else {
        viewport.bottom() + delta
    };
    let target = Pos2::new(viewport.left(), target_y);
    ui.scroll_to_rect(Rect::from_min_max(target, target), None);
    ui.ctx().request_repaint();
}

fn quick_access_auto_scroll_delta(pointer: Pos2, viewport: Rect, delta_seconds: f32) -> f32 {
    if pointer.x < viewport.left() || pointer.x > viewport.right() {
        return 0.0;
    }

    let margin = QUICK_ACCESS_AUTO_SCROLL_MARGIN.min(viewport.height() * 0.5);
    if margin <= 0.0 {
        return 0.0;
    }

    let (direction, penetration) = if pointer.y < viewport.top() + margin {
        (-1.0, viewport.top() + margin - pointer.y)
    } else if pointer.y > viewport.bottom() - margin {
        (1.0, pointer.y - (viewport.bottom() - margin))
    } else {
        return 0.0;
    };
    let edge_strength = (penetration / margin).clamp(0.0, 1.0);
    let speed = QUICK_ACCESS_AUTO_SCROLL_MAX_SPEED * edge_strength * edge_strength;
    let frame_delta = delta_seconds.clamp(0.0, QUICK_ACCESS_AUTO_SCROLL_MAX_FRAME_DELTA);
    direction * speed * frame_delta
}

#[cfg(test)]
#[path = "quick_access_items/tests.rs"]
mod tests;
