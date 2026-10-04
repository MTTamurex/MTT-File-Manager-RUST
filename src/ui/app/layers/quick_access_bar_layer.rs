use crate::app::navigation_state::QuickAccessPlacement;
use crate::app::ImageViewerApp;
use eframe::egui::{self, Color32, Pos2, Rect, Sense};
use rust_i18n::t;

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
        Color32::from_rgb(80, 80, 80)
    } else {
        Color32::from_rgb(210, 210, 210)
    };
    let folder_items: Vec<(String, String)> = app
        .pinned_folders
        .iter()
        .map(|folder| (folder.path.clone(), folder.display_name.clone()))
        .collect();
    let mut reorder = None;
    let mut navigate = None;
    let mut context_menu_path = None;
    let mut unpin_path = None;
    let mut pin_path = None;
    let mut pin_current_path = None;
    let mut cancel_drag_on_bar = false;

    egui::Panel::top("quick_access_bar")
        .show_separator_line(false)
        .exact_size(QUICK_ACCESS_BAR_HEIGHT)
        .frame(egui::Frame {
            fill: if dark_mode {
                Color32::from_rgb(45, 45, 45)
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
                                crate::ui::theme::selection_hover_color(dark_mode),
                            );
                        }
                        ui.painter().text(
                            add_rect.center(),
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
                            add_response =
                                add_response.on_hover_text(t!("context_menu.pin_quick_access"));
                            if add_response.clicked() {
                                pin_current_path = Some(current_path);
                            }
                        }

                        if folder_items.is_empty() && !app.show_recycle_bin {
                            ui.label(
                                egui::RichText::new(t!("sidebar.quick_access"))
                                    .color(ui.visuals().weak_text_color()),
                            );
                        }

                        let mut recycle_bin_rect = None;
                        if app.show_recycle_bin {
                            let label = t!("nav.recycle_bin").to_string();
                            let width = chip_width(ui, &label);
                            let (rect, response) =
                                ui.allocate_exact_size(egui::vec2(width, 26.0), Sense::click());
                            recycle_bin_rect = Some(rect);
                            let icon = app.item_icon_loader.ensure_recycle_bin_icon(ui.ctx());
                            paint_chip(
                                ui,
                                rect,
                                &label,
                                icon.as_ref(),
                                app.navigation_state.is_recycle_bin_view,
                                response.hovered(),
                            );
                            if response.clicked() && app.renaming_state.is_none() {
                                navigate = Some(None);
                            }
                        }

                        let drag_id = egui::Id::new("qa_bar_reorder_drag");
                        let drag_src: Option<usize> = ui.ctx().data(|data| data.get_temp(drag_id));
                        let pointer = ui.input(|input| input.pointer.hover_pos());
                        let viewport = ui.clip_rect();
                        let mut pointer_over_item = None;
                        let pointer_over_recycle_bin = pointer.is_some_and(|position| {
                            recycle_bin_rect.is_some_and(|rect| rect.contains(position))
                        });
                        let pointer_before_first_item = pointer.is_some_and(|position| {
                            !pointer_over_recycle_bin
                                && viewport.contains(position)
                                && (folder_items.is_empty() || position.x < ui.cursor().min.x)
                        });

                        for (index, (path, display_name)) in folder_items.iter().enumerate() {
                            let label =
                                crate::infrastructure::onedrive::special_folder_display_name(
                                    std::path::Path::new(path),
                                )
                                .unwrap_or_else(|| display_name.clone());
                            let width = chip_width(ui, &label);
                            let label_font = egui::FontId::proportional(12.0);
                            let visible_label =
                                crate::ui::preview_panel::utils::truncate_text_to_fit(
                                    &label,
                                    (width - 56.0).max(1.0),
                                    &label_font,
                                    ui,
                                );
                            let (rect, response) =
                                ui.allocate_exact_size(egui::vec2(width, 26.0), Sense::hover());
                            let pin_size = 22.0;
                            let pin_rect = Rect::from_center_size(
                                Pos2::new(rect.right() - pin_size / 2.0 - 2.0, rect.center().y),
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
                                .interact(pin_rect, response.id.with("unpin"), Sense::click())
                                .on_hover_text(t!("sidebar.remove_from_quick_access"));
                            if let Some(position) =
                                pointer.filter(|position| rect.contains(*position))
                            {
                                let after_item = position.x >= rect.center().x;
                                let insertion_index = index + if after_item { 1 } else { 0 };
                                pointer_over_item = Some(insertion_index);
                                if drag_src.is_some() {
                                    let marker_x = if after_item {
                                        rect.right()
                                    } else {
                                        rect.left()
                                    };
                                    ui.painter().vline(
                                        marker_x,
                                        rect.y_range(),
                                        egui::Stroke::new(2.0, Color32::from_rgb(0, 120, 215)),
                                    );
                                }
                            }
                            if item_response.drag_started() {
                                ui.ctx().data_mut(|data| data.insert_temp(drag_id, index));
                            }

                            let selected = !app.navigation_state.is_computer_view
                                && !app.navigation_state.is_recycle_bin_view
                                && app.navigation_state.current_path == *path;
                            let icon = app
                                .item_icon_loader
                                .get_or_load_registered_folder_icon(ui.ctx(), path)
                                .or_else(|| {
                                    app.item_icon_loader
                                        .get_or_load_folder_path_icon(ui.ctx(), path)
                                });
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

                            if item_response.clicked()
                                && !item_response.dragged()
                                && app.renaming_state.is_none()
                            {
                                navigate = Some(Some(path.clone()));
                            }
                            if pin_response.clicked() {
                                unpin_path = Some(path.clone());
                            }
                            if item_response.secondary_clicked() || pin_response.secondary_clicked()
                            {
                                context_menu_path = Some(path.clone());
                            }
                        }

                        if let Some(source) = drag_src {
                            if pointer_over_item.is_none()
                                && !pointer_over_recycle_bin
                                && pointer.is_some_and(|position| viewport.contains(position))
                            {
                                ui.painter().vline(
                                    ui.cursor().min.x,
                                    viewport.y_range(),
                                    egui::Stroke::new(2.0, Color32::from_rgb(0, 120, 215)),
                                );
                            }
                            let released = ui.ctx().input(|input| input.pointer.primary_released());
                            let cancelled = ui.ctx().input(|input| {
                                input.key_pressed(egui::Key::Escape)
                                    || !input.viewport().focused.unwrap_or(true)
                                    || (!input.pointer.primary_down() && !released)
                            });
                            if cancelled || released {
                                ui.ctx().data_mut(|data| data.remove::<usize>(drag_id));
                            }
                            if released
                                && pointer.is_some_and(|position| viewport.contains(position))
                            {
                                let insertion_index =
                                    pointer_over_item.unwrap_or(if pointer_before_first_item {
                                        0
                                    } else {
                                        folder_items.len()
                                    });
                                let destination = reorder_destination(
                                    source,
                                    insertion_index,
                                    folder_items.len(),
                                );
                                if destination != source {
                                    reorder = Some((source, destination));
                                }
                            } else if let Some(position) = pointer.filter(|position| {
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
                                    cancel_drag_on_bar = true;
                                    if is_folder_drop {
                                        pin_path = app
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
                    });
                });
        });

    if cancel_drag_on_bar {
        app.cancel_item_drag();
    }
    if let Some(target) = navigate {
        match target {
            Some(path) => app.navigate_to(&path),
            None => app.navigate_to_recycle_bin(),
        }
    }
    if let Some((from, to)) = reorder {
        app.reorder_pinned_folder(from, to);
    }
    if let Some(path) = unpin_path {
        app.unpin_folder(&path);
    }
    if let Some(path) = pin_path {
        app.pin_folder(&path);
    }
    if let Some(path) = pin_current_path {
        app.pin_folder(&path);
    }
    if let Some(path) = context_menu_path {
        let path_buf = std::path::PathBuf::from(path);
        let position = root_ui
            .ctx()
            .input(|input| input.pointer.hover_pos().unwrap_or_default());
        app.context_menu
            .open(position, None, vec![path_buf.clone()], false);
        app.context_menu.primary_is_directory = Some(true);
        app.populate_context_menu(root_ui.ctx(), &[path_buf], false, None);
    }
}

fn reorder_destination(source: usize, insertion_index: usize, item_count: usize) -> usize {
    let insertion_index = insertion_index.min(item_count);
    insertion_index.saturating_sub(if insertion_index > source { 1 } else { 0 })
}

fn chip_width(ui: &egui::Ui, label: &str) -> f32 {
    let galley = ui.painter().layout_no_wrap(
        label.to_string(),
        egui::FontId::proportional(12.0),
        ui.visuals().text_color(),
    );
    (galley.size().x + 56.0).clamp(96.0, 240.0)
}

fn paint_chip(
    ui: &egui::Ui,
    rect: Rect,
    label: &str,
    icon: Option<&egui::TextureHandle>,
    selected: bool,
    hovered: bool,
) {
    if !ui.is_rect_visible(rect) {
        return;
    }

    let dark_mode = ui.visuals().dark_mode;
    let fill = if selected {
        crate::ui::theme::selection_color(dark_mode)
    } else if hovered {
        crate::ui::theme::selection_hover_color(dark_mode)
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 5.0, fill);

    let mut text_x = rect.left() + 10.0;
    if let Some(icon) = icon {
        let icon_rect = Rect::from_center_size(
            Pos2::new(text_x + 8.0, rect.center().y),
            egui::vec2(16.0, 16.0),
        );
        ui.painter().image(
            icon.id(),
            icon_rect,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
        text_x += 22.0;
    } else {
        text_x += 22.0;
    }
    ui.painter().text(
        Pos2::new(text_x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(12.0),
        if selected {
            crate::ui::theme::selection_text_color(dark_mode)
        } else {
            ui.visuals().text_color()
        },
    );
}

#[cfg(test)]
mod tests {
    use super::reorder_destination;

    #[test]
    fn reorder_destination_maps_horizontal_gaps_to_final_indices() {
        assert_eq!(reorder_destination(0, 0, 4), 0);
        assert_eq!(reorder_destination(0, 1, 4), 0);
        assert_eq!(reorder_destination(0, 2, 4), 1);
        assert_eq!(reorder_destination(2, 1, 4), 1);
        assert_eq!(reorder_destination(2, 3, 4), 2);
        assert_eq!(reorder_destination(2, 4, 4), 3);
    }

    /// Regression test: the row wrappers used before pushed the chips 4px down,
    /// overflowing the panel bottom margin. Chips and the "+" slot must stay
    /// strictly between the panel margin top/bottom.
    #[test]
    fn quick_access_bar_row_stays_inside_panel_margins() {
        use eframe::egui;
        use eframe::egui::scroll_area::ScrollBarVisibility;
        use eframe::egui::{pos2, vec2, Color32, Margin, RawInput, Sense, Ui};
        use std::cell::Cell;

        let ctx = eframe::egui::Context::default();
        let bar_range = Cell::new((f32::NAN, f32::NAN));
        let chip_range = Cell::new((f32::NAN, f32::NAN));
        let add_range = Cell::new((f32::NAN, f32::NAN));

        let _ = ctx.run_ui(
            RawInput {
                screen_rect: Some(eframe::egui::Rect::from_min_size(
                    pos2(0.0, 0.0),
                    vec2(800.0, 600.0),
                )),
                ..RawInput::default()
            },
            |ui: &mut Ui| {
                egui::Panel::top("quick_access_bar")
                    .show_separator_line(false)
                    .exact_size(34.0)
                    .frame(egui::Frame {
                        fill: Color32::from_rgb(243, 243, 243),
                        inner_margin: Margin {
                            left: 8,
                            right: 8,
                            top: 3,
                            bottom: 3,
                        },
                        ..Default::default()
                    })
                    .show(ui, |panel_ui: &mut Ui| {
                        let bar_rect = panel_ui.max_rect();
                        bar_range.set((bar_rect.top(), bar_rect.bottom()));

                        let scroll_output = egui::ScrollArea::horizontal()
                            .id_salt("quick_access_bar_scroll")
                            .auto_shrink([false, false])
                            .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
                            .show(panel_ui, |scroll_ui: &mut Ui| {
                                scroll_ui.horizontal(|row_ui: &mut Ui| {
                                    row_ui.spacing_mut().item_spacing.x = 4.0;
                                    let (add_rect, _) = row_ui
                                        .allocate_exact_size(vec2(26.0, 26.0), Sense::click());
                                    add_range.set((add_rect.top(), add_rect.bottom()));
                                    let (chip_rect, _) = row_ui
                                        .allocate_exact_size(vec2(120.0, 26.0), Sense::hover());
                                    chip_range.set((chip_rect.top(), chip_rect.bottom()));
                                });
                            });
                        assert!(!scroll_output.inner_rect.is_negative());
                    });
            },
        );

        let (bar_top, bar_bottom) = bar_range.get();
        let (chip_top, chip_bottom) = chip_range.get();
        let (add_top, add_bottom) = add_range.get();
        assert!(chip_top >= bar_top, "chip overlaps top margin");
        assert!(chip_bottom <= bar_bottom, "chip overflows bottom margin");
        assert!(add_top >= bar_top, "+ slot overlaps top margin");
        assert!(add_bottom <= bar_bottom, "+ slot overflows bottom margin");
    }
}
