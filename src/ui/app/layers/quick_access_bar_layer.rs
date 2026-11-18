use crate::app::navigation_state::QuickAccessPlacement;
use crate::app::ImageViewerApp;
use crate::domain::quick_access_item::{
    destination_for_quick_access_gap, ordered_quick_access_items, QuickAccessItem,
};
use eframe::egui::{self, Color32, Pos2, Rect, Sense};
use rust_i18n::t;

mod actions;
mod chips;
#[cfg(test)]
mod tests;

use chips::{chip_label, paint_chip, paint_group_chip, CHIP_WIDTH};

pub(crate) const QUICK_ACCESS_BAR_HEIGHT: f32 = 34.0;
pub(crate) fn quick_access_bar_visible(app: &ImageViewerApp) -> bool {
    app.show_quick_access && app.quick_access_placement == QuickAccessPlacement::Horizontal
}

pub(crate) fn render_quick_access_bar_layer(app: &mut ImageViewerApp, root_ui: &mut egui::Ui) {
    if !quick_access_bar_visible(app) {
        root_ui
            .ctx()
            .data_mut(|data| data.remove::<usize>(egui::Id::new("qa_bar_reorder_drag")));
        return;
    }

    let dark_mode = root_ui.visuals().dark_mode;
    let separator_color = if dark_mode {
        crate::ui::theme::palette::border(root_ui.ctx(), Color32::from_rgb(80, 80, 80))
    } else {
        Color32::from_rgb(210, 210, 210)
    };
    let folder_items: Vec<(String, String)> = app
        .pinned_folders
        .iter()
        .map(|folder| (folder.path.clone(), folder.display_name.clone()))
        .collect();
    let group_items: Vec<(i64, String, usize)> = app
        .folder_groups
        .iter()
        .map(|group| (group.id, group.name.clone(), group.paths.len()))
        .collect();
    let quick_access_items = ordered_quick_access_items(&app.pinned_folders, &app.folder_groups);
    let mut deferred = actions::DeferredActions::default();
    let can_drop_items_to_recycle_bin = app.can_drop_drag_payload_to_recycle_bin();

    egui::Panel::top("quick_access_bar")
        .show_separator_line(false)
        .exact_size(QUICK_ACCESS_BAR_HEIGHT)
        .frame(egui::Frame {
            fill: if dark_mode {
                crate::ui::theme::palette::surface(root_ui.ctx(), Color32::from_rgb(45, 45, 45))
            } else {
                Color32::from_rgb(243, 243, 243)
            },
            inner_margin: egui::Margin {
                left: 8,
                right: 8,
                top: 3,
                bottom: 3,
            },
            ..Default::default()
        })
        .show(root_ui, |ui| {
            let bar_rect = ui.max_rect();
            let separator_stroke = egui::Stroke::new(1.0, separator_color);
            // The address bar's native panel separator is disabled while this
            // bar is visible, so both edges here get the same light stroke:
            // matching the line between this bar and the command bar below.
            ui.painter()
                .hline(bar_rect.x_range(), bar_rect.bottom(), separator_stroke);
            ui.painter()
                .hline(bar_rect.x_range(), bar_rect.top(), separator_stroke);

            egui::ScrollArea::horizontal()
                .id_salt("quick_access_bar_scroll")
                .auto_shrink([false, false])
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let pointer = ui.input(|input| input.pointer.hover_pos());
                        let viewport = ui.clip_rect();

                        let current_path = app.navigation_state.current_path.clone();
                        let current_path_is_pinned = app
                            .pinned_folders
                            .iter()
                            .any(|folder| folder.path == current_path);
                        let can_pin_current_path = !current_path.is_empty()
                            && !crate::domain::special_paths::is_virtual_path(&current_path)
                            && !current_path_is_pinned;
                        let add_sense = if can_pin_current_path {
                            Sense::click()
                        } else {
                            Sense::hover()
                        };
                        let (add_rect, mut add_response) =
                            ui.allocate_exact_size(egui::vec2(26.0, 26.0), add_sense);
                        if add_response.hovered() && can_pin_current_path {
                            ui.painter().rect_filled(
                                add_rect,
                                5.0,
                                crate::ui::theme::selection_hover_color(ui.ctx()),
                            );
                        }
                        // Optically the 20pt "+" sits 1px low; nudge the glyph up.
                        ui.painter().text(
                            egui::Pos2::new(add_rect.center().x, add_rect.center().y - 1.0),
                            egui::Align2::CENTER_CENTER,
                            "+",
                            egui::FontId::proportional(20.0),
                            if can_pin_current_path {
                                ui.visuals().text_color()
                            } else {
                                ui.visuals().weak_text_color()
                            },
                        );
                        if can_pin_current_path {
                            add_response = add_response
                                .on_hover_text(t!("quick_access_bar.pin_current_folder_tooltip"));
                            if add_response.clicked() {
                                deferred.pin_current_path = Some(current_path);
                            }
                        }

                        if folder_items.is_empty()
                            && group_items.is_empty()
                            && !app.show_recycle_bin
                        {
                            ui.label(
                                egui::RichText::new(t!("sidebar.quick_access"))
                                    .color(ui.visuals().weak_text_color()),
                            );
                        }

                        let mut recycle_bin_rect = None;
                        if app.show_recycle_bin {
                            let label = t!("nav.recycle_bin").to_string();
                            let visible_label = chip_label(ui, &label);
                            let (rect, response) = ui
                                .allocate_exact_size(egui::vec2(CHIP_WIDTH, 26.0), Sense::click());
                            let response = response.on_hover_text(&label);
                            recycle_bin_rect = Some(rect);
                            let pointer_over_recycle_bin =
                                pointer.is_some_and(|position| rect.contains(position));
                            let can_drop_here =
                                can_drop_items_to_recycle_bin && pointer_over_recycle_bin;
                            let icon = app.item_icon_loader.ensure_recycle_bin_icon(ui.ctx());
                            paint_chip(
                                ui,
                                rect,
                                &visible_label,
                                icon.as_ref(),
                                app.navigation_state.is_recycle_bin_view,
                                response.hovered() || can_drop_here,
                            );
                            if can_drop_here {
                                ui.painter().rect_stroke(
                                    rect,
                                    5.0,
                                    egui::Stroke::new(2.0, Color32::from_rgb(24, 122, 255)),
                                    egui::StrokeKind::Inside,
                                );
                            }
                            if response.clicked()
                                && !app.is_item_dragging
                                && app.renaming_state.is_none()
                            {
                                deferred.navigate = Some(None);
                            }
                            if response.middle_clicked()
                                && !app.is_item_dragging
                                && app.renaming_state.is_none()
                            {
                                deferred.new_tab_path = Some(
                                    crate::domain::special_paths::RECYCLE_BIN_VIEW_ID.to_string(),
                                );
                            }
                        }

                        let drag_id = egui::Id::new("qa_bar_reorder_drag");
                        let drag_src: Option<usize> = ui.ctx().data(|data| data.get_temp(drag_id));
                        let pointer_over_recycle_bin = pointer.is_some_and(|position| {
                            recycle_bin_rect.is_some_and(|rect| rect.contains(position))
                        });
                        let mut item_rects = Vec::with_capacity(quick_access_items.len());
                        let mut source_owns_drag = false;
                        let mut preview = crate::ui::tab_drag_preview::TabDragPreview::default();

                        for (index, item) in quick_access_items.iter().enumerate() {
                            let paint_start =
                                crate::ui::tab_drag_preview::TabDragPreview::begin(ui);
                            match item {
                                QuickAccessItem::PinnedFolder(path) => {
                                    let Some((_, display_name)) =
                                        folder_items.iter().find(|(item_path, _)| item_path == path)
                                    else {
                                        continue;
                                    };
                                    let label =
                                        crate::infrastructure::onedrive::special_folder_display_name(
                                            std::path::Path::new(path),
                                        )
                                        .unwrap_or_else(|| display_name.clone());
                                    let visible_label = chip_label(ui, &label);
                                    let (rect, response) = ui.allocate_exact_size(
                                        egui::vec2(CHIP_WIDTH, 26.0),
                                        Sense::hover(),
                                    );
                                    let pin_size = 22.0;
                                    let pin_rect = Rect::from_center_size(
                                        Pos2::new(
                                            rect.right() - pin_size / 2.0 - 2.0,
                                            rect.center().y,
                                        ),
                                        egui::vec2(pin_size, pin_size),
                                    );
                                    let item_rect = Rect::from_min_max(
                                        rect.min,
                                        Pos2::new(pin_rect.left(), rect.max.y),
                                    );
                                    let item_response = ui.interact(
                                        item_rect,
                                        response.id.with("navigate"),
                                        Sense::click_and_drag(),
                                    );
                                    let pin_response = ui
                                        .interact(
                                            pin_rect,
                                            response.id.with("unpin"),
                                            Sense::click(),
                                        )
                                        .on_hover_text(t!("sidebar.remove_from_quick_access"));
                                    if item_response.drag_started() {
                                        ui.ctx().data_mut(|data| {
                                            data.insert_temp(drag_id, index)
                                        });
                                    }
                                    if drag_src == Some(index) {
                                        source_owns_drag = item_response
                                            .dragged_by(egui::PointerButton::Primary)
                                            || item_response.drag_stopped_by(
                                                egui::PointerButton::Primary,
                                            );
                                    }

                                    let selected = !app.navigation_state.is_computer_view
                                        && !app.navigation_state.is_recycle_bin_view
                                        && app.navigation_state.current_path == *path;
                                    let icon = app
                                        .item_icon_loader
                                        .get_or_load_folder_navigation_icon(ui.ctx(), path);
                                    paint_chip(
                                        ui,
                                        rect,
                                        &visible_label,
                                        icon.as_ref(),
                                        selected,
                                        item_response.hovered()
                                            || item_response.dragged()
                                            || pin_response.hovered(),
                                    );
                                    let item_response = if visible_label != label {
                                        item_response.on_hover_text(&label)
                                    } else {
                                        item_response.on_hover_text(path)
                                    };
                                    let pin_color = if pin_response.hovered() {
                                        Color32::from_rgb(220, 60, 60)
                                    } else {
                                        Color32::from_gray(140)
                                    };
                                    ui.painter().text(
                                        pin_rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        "📌",
                                        egui::FontId::proportional(12.0),
                                        pin_color,
                                    );
                                    preview.record(ui, rect, paint_start);
                                    item_rects.push(rect);

                                    if item_response.clicked()
                                        && !item_response.dragged()
                                        && app.renaming_state.is_none()
                                    {
                                        deferred.navigate = Some(Some(path.clone()));
                                    }
                                    if item_response.middle_clicked()
                                        && app.renaming_state.is_none()
                                    {
                                        deferred.new_tab_path = Some(path.clone());
                                    }
                                    if pin_response.clicked() {
                                        deferred.unpin_path = Some(path.clone());
                                    }
                                    if item_response.secondary_clicked()
                                        || pin_response.secondary_clicked()
                                    {
                                        deferred.context_menu_path = Some(path.clone());
                                    }
                                }
                                QuickAccessItem::FolderGroup(group_id) => {
                                    let Some((_, name, path_count)) =
                                        group_items.iter().find(|(id, _, _)| id == group_id)
                                    else {
                                        continue;
                                    };
                                    let label = chip_label(ui, name);
                                    let (rect, response) = ui.allocate_exact_size(
                                        egui::vec2(CHIP_WIDTH, 26.0),
                                        Sense::click_and_drag(),
                                    );
                                    let response = response.on_hover_text(
                                        t!("sidebar.group_open_count", count = path_count),
                                    );
                                    if response.drag_started() {
                                        ui.ctx().data_mut(|data| {
                                            data.insert_temp(drag_id, index)
                                        });
                                    }
                                    if drag_src == Some(index) {
                                        source_owns_drag = response
                                            .dragged_by(egui::PointerButton::Primary)
                                            || response.drag_stopped_by(
                                                egui::PointerButton::Primary,
                                            );
                                    }
                                    paint_group_chip(
                                        ui,
                                        rect,
                                        &label,
                                        response.hovered() || response.dragged(),
                                    );
                                    preview.record(ui, rect, paint_start);
                                    item_rects.push(rect);
                                    if response.clicked()
                                        && !response.dragged()
                                        && !app.is_item_dragging
                                        && app.renaming_state.is_none()
                                    {
                                        deferred.open_group_id = Some(*group_id);
                                    }
                                    response.context_menu(|ui| {
                                        if ui.button(t!("sidebar.group_open_all")).clicked() {
                                            deferred.open_group_id = Some(*group_id);
                                            ui.close();
                                        }
                                        ui.separator();
                                        if ui.button(t!("sidebar.group_delete")).clicked() {
                                            deferred.delete_group_id = Some(*group_id);
                                            ui.close();
                                        }
                                    });
                                }
                            }
                        }

                        if let Some(source) = drag_src {
                            let released = ui.ctx().input(|input| input.pointer.primary_released());
                            let cancelled = ui.ctx().input(|input| {
                                input.key_pressed(egui::Key::Escape)
                                    || !input.viewport().focused.unwrap_or(true)
                                    || (!input.pointer.primary_down() && !released)
                            });
                            let pointer_in_viewport =
                                pointer.is_some_and(|position| viewport.contains(position));
                            let insertion_gap = if pointer_over_recycle_bin {
                                None
                            } else {
                                horizontal_drop_gap(pointer, &item_rects, viewport)
                            };
                            preview.paint_at_gap(
                                ui,
                                drag_id.with("preview"),
                                (!cancelled && source_owns_drag).then_some(source),
                                viewport,
                                insertion_gap,
                            );
                            if cancelled || released {
                                ui.ctx().data_mut(|data| data.remove::<usize>(drag_id));
                            }
                            if released
                                && !cancelled
                                && source_owns_drag
                                && pointer_in_viewport
                                && !pointer_over_recycle_bin
                            {
                                let insertion_index =
                                    insertion_gap.unwrap_or(item_rects.len());
                                let destination = destination_for_quick_access_gap(
                                    source,
                                    insertion_index,
                                    item_rects.len(),
                                );
                                if destination != source {
                                    deferred.reorder = Some((source, destination));
                                }
                            } else if let Some(position) = pointer.filter(|position| {
                                !released
                                    &&
                                viewport.y_range().contains(position.y)
                                    && (position.x < viewport.left() + 28.0
                                        || position.x > viewport.right() - 28.0)
                            }) {
                                let target_x = if position.x < viewport.center().x {
                                    viewport.left() - 12.0
                                } else {
                                    viewport.right() + 12.0
                                };
                                let target = Pos2::new(target_x, viewport.center().y);
                                ui.scroll_to_rect(Rect::from_min_max(target, target), None);
                                ui.ctx().request_repaint();
                            }
                        }

                        if app.is_item_dragging {
                            let pointer_pos = ui.ctx().input(|input| input.pointer.hover_pos());
                            if pointer_pos.is_some_and(|position| bar_rect.contains(position)) {
                                let is_folder_drop = app.drag_payload_is_single_directory;
                                if is_folder_drop {
                                    ui.painter().rect_filled(
                                        bar_rect,
                                        0.0,
                                        Color32::from_rgba_premultiplied(100, 120, 215, 24),
                                    );
                                }
                                let released =
                                    ui.ctx().input(|input| input.pointer.primary_released());
                                if released {
                                    if can_drop_items_to_recycle_bin && pointer_over_recycle_bin {
                                        deferred.drop_to_recycle_bin = true;
                                    } else {
                                        deferred.cancel_drag_on_bar = true;
                                        if is_folder_drop {
                                            deferred.pin_path = app
                                                .drag_payload_paths
                                                .first()
                                                .and_then(|path| path.to_str())
                                                .filter(|path| {
                                                    !folder_items
                                                        .iter()
                                                        .any(|(pinned, _)| pinned == *path)
                                                })
                                                .map(str::to_string);
                                        }
                                    }
                                }
                            }
                        }
                    });
                });
        });

    actions::apply(app, root_ui, deferred);
}

fn horizontal_drop_gap(pointer: Option<Pos2>, rects: &[Rect], viewport: Rect) -> Option<usize> {
    let pointer = pointer.filter(|position| viewport.contains(*position))?;
    Some(
        rects
            .iter()
            .position(|rect| pointer.x < rect.center().x)
            .unwrap_or(rects.len()),
    )
}
