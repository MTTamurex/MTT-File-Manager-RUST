use crate::app::ImageViewerApp;
use crate::domain::file_entry::{SortMode, ViewMode};
use crate::ui::theme;
use crate::ui::widgets;
use eframe::egui;

pub(crate) fn render_secondary_toolbar_layer(app: &mut ImageViewerApp, ctx: &egui::Context) {
    let separator_color = if ctx.style().visuals.dark_mode {
        egui::Color32::from_rgb(80, 80, 80)
    } else {
        egui::Color32::from_rgb(210, 210, 210)
    };

    egui::TopBottomPanel::top("secondary_nav_bar")
        .show_separator_line(false)
        .exact_height(46.0)
        .frame(egui::Frame {
            fill: if ctx.style().visuals.dark_mode {
                egui::Color32::from_rgb(45, 45, 45)
            } else {
                egui::Color32::WHITE
            },
            inner_margin: egui::Margin {
                left: 8,
                right: 8,
                top: 7,
                bottom: 7,
            },
            ..Default::default()
        })
        .show(ctx, |ui| {
            let rect = ui.max_rect();
            ui.painter().hline(
                rect.x_range(),
                rect.bottom(),
                egui::Stroke::new(1.0, separator_color),
            );

            enum SecAction {
                None,
                Cut,
                Copy,
                Paste,
                Rename,
                CreateFolder,
                Delete,
            }
            let mut action = SecAction::None;

            ui.horizontal(|ui| {
                let content_width =
                    6.0 * 28.0 + 30.0 + 110.0 + 2.0 * 28.0 + 80.0 + 80.0 + 3.0 * 8.0 + 16.0 * 12.0;
                let available = ui.available_width();
                let left_pad = ((available - content_width) / 2.0).max(0.0);
                ui.add_space(left_pad);

                ui.spacing_mut().item_spacing = egui::vec2(12.0, 0.0);

                let icon_size = egui::vec2(28.0, 28.0);

                let is_drive_selected = app
                    .selected_file
                    .as_ref()
                    .is_some_and(|f| f.drive_info.is_some());
                let has_selection = (app.selected_file.is_some()
                    || !app.multi_selection.is_empty())
                    && !is_drive_selected;
                let can_rename = app.multi_selection.len() <= 1
                    && (app.multi_selection.len() == 1 || app.selected_file.is_some());
                let can_paste = app.clipboard.has_content() && !is_drive_selected;
                let can_create_folder = !app.is_computer_view && !app.is_recycle_bin_view;

                let icon_color = if ui.visuals().dark_mode {
                    [220, 220, 220, 255]
                } else {
                    [60, 60, 60, 255]
                };
                let disabled_color = [128, 128, 128, 180];

                {
                    let svg_manager = &mut app.svg_icon_manager;

                    let mut render_btn = |icon_name: &str, enabled: bool, tooltip: &str| -> bool {
                        let color = if enabled { icon_color } else { disabled_color };
                        let sense = if enabled {
                            egui::Sense::click()
                        } else {
                            egui::Sense::hover()
                        };
                        let (rect, response) = ui.allocate_exact_size(icon_size, sense);

                        if enabled && response.hovered() {
                            let bg_color = if ui.visuals().dark_mode {
                                theme::color_dark_hover()
                            } else {
                                theme::color_hover()
                            };
                            ui.painter().rect_filled(rect, 6.0, bg_color);
                        }

                        if let Some(texture) = svg_manager.get_icon(ui.ctx(), icon_name, 32, color)
                        {
                            let display_size = if icon_name == "folder_new" {
                                18.0
                            } else {
                                16.0
                            };
                            let icon_rect = egui::Rect::from_center_size(
                                rect.center(),
                                egui::vec2(display_size, display_size),
                            );
                            ui.painter().image(
                                texture.id(),
                                icon_rect,
                                egui::Rect::from_min_max(
                                    egui::pos2(0.0, 0.0),
                                    egui::pos2(1.0, 1.0),
                                ),
                                egui::Color32::WHITE,
                            );
                        } else {
                            let fallback = icon_name.chars().next().unwrap_or('?').to_string();
                            ui.painter().text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                fallback,
                                egui::FontId::proportional(12.0),
                                egui::Color32::from_rgba_unmultiplied(
                                    color[0], color[1], color[2], color[3],
                                ),
                            );
                        }

                        let response = if enabled {
                            response.on_hover_cursor(egui::CursorIcon::PointingHand)
                        } else {
                            response
                        };

                        if enabled {
                            response.on_hover_text(tooltip).clicked()
                        } else {
                            response.on_hover_text(format!("{} (Desabilitado)", tooltip));
                            false
                        }
                    };

                    if render_btn("cut", has_selection, "Recortar (Ctrl+X)") {
                        action = SecAction::Cut;
                    }
                    if render_btn("copy", has_selection, "Copiar (Ctrl+C)") {
                        action = SecAction::Copy;
                    }
                    if render_btn("paste", can_paste, "Colar (Ctrl+V)") {
                        action = SecAction::Paste;
                    }
                    if render_btn("rename", can_rename, "Renomear (F2)") {
                        action = SecAction::Rename;
                    }
                    if render_btn(
                        "folder_new",
                        can_create_folder,
                        "Criar Nova Pasta (Ctrl+Shift+N)",
                    ) {
                        action = SecAction::CreateFolder;
                    }
                    if render_btn("delete", has_selection, "Excluir (Del)") {
                        action = SecAction::Delete;
                    }
                }

                ui.separator();

                let sort_symbol = if app.sort_descending { "↓" } else { "↑" };

                ui.scope(|ui| {
                    let hover_color = if ui.visuals().dark_mode {
                        theme::color_dark_hover()
                    } else {
                        theme::color_hover()
                    };

                    ui.visuals_mut().widgets.inactive.bg_fill = egui::Color32::TRANSPARENT;
                    ui.visuals_mut().widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
                    ui.visuals_mut().widgets.inactive.fg_stroke = egui::Stroke::NONE;
                    ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::NONE;

                    ui.visuals_mut().widgets.hovered.bg_fill = hover_color;
                    ui.visuals_mut().widgets.hovered.fg_stroke = egui::Stroke::NONE;
                    ui.visuals_mut().widgets.hovered.bg_stroke = egui::Stroke::NONE;

                    if ui
                        .add(egui::Button::new(
                            egui::RichText::new(sort_symbol).color(egui::Color32::BLACK),
                        ))
                        .on_hover_text("Inverter Ordem")
                        .clicked()
                    {
                        app.sort_descending = !app.sort_descending;
                        app.sort_items();
                        app.save_preferences();
                    }
                });

                ui.scope(|ui| {
                    let hover_color = if ui.visuals().dark_mode {
                        theme::color_dark_hover()
                    } else {
                        theme::color_hover()
                    };

                    let black_stroke = egui::Stroke::new(1.0, egui::Color32::BLACK);

                    ui.visuals_mut().widgets.noninteractive.bg_fill = egui::Color32::WHITE;
                    ui.visuals_mut().widgets.noninteractive.fg_stroke = black_stroke;
                    ui.visuals_mut().widgets.noninteractive.bg_stroke = egui::Stroke::NONE;

                    ui.visuals_mut().widgets.inactive.bg_fill = egui::Color32::WHITE;
                    ui.visuals_mut().widgets.inactive.fg_stroke = black_stroke;
                    ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::NONE;

                    ui.visuals_mut().widgets.hovered.bg_fill = hover_color;
                    ui.visuals_mut().widgets.hovered.fg_stroke = black_stroke;
                    ui.visuals_mut().widgets.hovered.bg_stroke = egui::Stroke::NONE;

                    ui.visuals_mut().widgets.active.bg_fill = hover_color;
                    ui.visuals_mut().widgets.active.fg_stroke = black_stroke;
                    ui.visuals_mut().widgets.active.bg_stroke = egui::Stroke::NONE;

                    ui.visuals_mut().override_text_color = Some(egui::Color32::BLACK);

                    egui::ComboBox::from_id_salt("sort_mode_secondary")
                        .selected_text(match app.sort_mode {
                            SortMode::Name => "Nome",
                            SortMode::Date => "Data",
                            SortMode::Size => "Tamanho",
                            SortMode::Type => "Tipo",
                            SortMode::DriveTotalSpace => "Espaço Total",
                            SortMode::DriveFreeSpace => "Espaço Livre",
                        })
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_value(&mut SortMode::Name, app.sort_mode, "Nome")
                                .clicked()
                            {
                                app.sort_mode = SortMode::Name;
                                if app.is_computer_view {
                                    app.sort_mode_computer = SortMode::Name;
                                } else {
                                    app.sort_mode_normal = SortMode::Name;
                                }
                                app.sort_items();
                                app.save_preferences();
                            }

                            if app.is_computer_view {
                                if ui
                                    .selectable_value(
                                        &mut SortMode::DriveTotalSpace,
                                        app.sort_mode,
                                        "Espaço Total",
                                    )
                                    .clicked()
                                {
                                    app.sort_mode = SortMode::DriveTotalSpace;
                                    app.sort_mode_computer = SortMode::DriveTotalSpace;
                                    app.sort_items();
                                    app.save_preferences();
                                }
                                if ui
                                    .selectable_value(
                                        &mut SortMode::DriveFreeSpace,
                                        app.sort_mode,
                                        "Espaço Livre",
                                    )
                                    .clicked()
                                {
                                    app.sort_mode = SortMode::DriveFreeSpace;
                                    app.sort_mode_computer = SortMode::DriveFreeSpace;
                                    app.sort_items();
                                    app.save_preferences();
                                }
                            } else {
                                if ui
                                    .selectable_value(&mut SortMode::Date, app.sort_mode, "Data")
                                    .clicked()
                                {
                                    app.sort_mode = SortMode::Date;
                                    app.sort_mode_normal = SortMode::Date;
                                    app.sort_items();
                                    app.save_preferences();
                                }
                                if ui
                                    .selectable_value(&mut SortMode::Size, app.sort_mode, "Tamanho")
                                    .clicked()
                                {
                                    app.sort_mode = SortMode::Size;
                                    app.sort_mode_normal = SortMode::Size;
                                    app.sort_items();
                                    app.save_preferences();
                                }
                                if ui
                                    .selectable_value(&mut SortMode::Type, app.sort_mode, "Tipo")
                                    .clicked()
                                {
                                    app.sort_mode = SortMode::Type;
                                    app.sort_mode_normal = SortMode::Type;
                                    app.sort_items();
                                    app.save_preferences();
                                }
                            }
                        });
                });

                ui.separator();

                {
                    let svg_manager = &mut app.svg_icon_manager;
                    if widgets::toggle_icon_button(
                        ui,
                        svg_manager,
                        theme::ICON_LIST,
                        matches!(app.view_mode, ViewMode::List),
                        "Lista",
                    )
                    .clicked()
                    {
                        if !matches!(app.view_mode, ViewMode::List) {
                            app.view_mode = ViewMode::List;
                        }
                    }

                    if widgets::toggle_icon_button(
                        ui,
                        svg_manager,
                        theme::ICON_GRID,
                        matches!(app.view_mode, ViewMode::Grid),
                        "Grade",
                    )
                    .clicked()
                    {
                        if !matches!(app.view_mode, ViewMode::Grid) {
                            app.view_mode = ViewMode::Grid;
                        }
                    }
                }

                ui.separator();

                ui.add_sized(
                    egui::vec2(80.0, 20.0),
                    egui::Slider::new(&mut app.thumbnail_size, 64.0..=256.0).show_value(false),
                );
                ui.label("Zoom");
            });

            match action {
                SecAction::Cut => app.command_cut(Option::from(app.selected_item)),
                SecAction::Copy => app.command_copy(Option::from(app.selected_item)),
                SecAction::Paste => app.command_paste(None),
                SecAction::Rename => {
                    if let Some(idx) = app.selected_item {
                        if let Some(item) = app.items.get(idx) {
                            app.renaming_state = Some((idx, item.name.clone()));
                            app.focus_rename = true;
                        }
                    }
                }
                SecAction::CreateFolder => app.create_new_folder(),
                SecAction::Delete => {
                    let mut targets = Vec::new();
                    if app.multi_selection.is_empty() {
                        if let Some(idx) = app.selected_item {
                            if let Some(item) = app.items.get(idx) {
                                targets.push(item.path.clone());
                            }
                        }
                    } else {
                        targets.extend(app.multi_selection.iter().cloned());
                    }

                    if !targets.is_empty() {
                        app.delete_with_shell_for_paths(&targets);
                    }
                }
                SecAction::None => {}
            }
        });
}
