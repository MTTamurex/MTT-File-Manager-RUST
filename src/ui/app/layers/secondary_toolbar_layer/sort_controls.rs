use crate::app::ImageViewerApp;
use crate::domain::file_entry::{FoldersPosition, SortMode};
use crate::ui::theme;
use eframe::egui;
use rust_i18n::t;

pub(super) fn render_sort_controls(ui: &mut egui::Ui, app: &mut ImageViewerApp) {
    let locked = app.current_folder_locked;
    let sort_symbol = if app.sort_descending {
        "\u{2193}"
    } else {
        "\u{2191}"
    };

    ui.scope(|ui| {
        if locked {
            ui.disable();
        }
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
                egui::RichText::new(sort_symbol).color(theme::text_color(ui.visuals().dark_mode)),
            ))
            .on_hover_text(t!("secondary_toolbar.reverse_order"))
            .clicked()
        {
            app.sort_descending = !app.sort_descending;
            if !app.current_folder_locked {
                app.sort_descending_normal = app.sort_descending;
            }
            app.sort_items();
            app.save_preferences();
        }
    });

    ui.scope(|ui| {
        if locked {
            ui.disable();
        }
        let hover_color = if ui.visuals().dark_mode {
            theme::color_dark_hover()
        } else {
            theme::color_hover()
        };

        let is_dark = ui.visuals().dark_mode;
        let fg_color = theme::text_color(is_dark);
        let combo_bg = theme::input_bg_color(is_dark);
        let fg_stroke = egui::Stroke::new(1.0, fg_color);

        ui.visuals_mut().widgets.noninteractive.bg_fill = combo_bg;
        ui.visuals_mut().widgets.noninteractive.fg_stroke = fg_stroke;
        ui.visuals_mut().widgets.noninteractive.bg_stroke = egui::Stroke::NONE;

        ui.visuals_mut().widgets.inactive.bg_fill = combo_bg;
        ui.visuals_mut().widgets.inactive.fg_stroke = fg_stroke;
        ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::NONE;

        ui.visuals_mut().widgets.hovered.bg_fill = hover_color;
        ui.visuals_mut().widgets.hovered.fg_stroke = fg_stroke;
        ui.visuals_mut().widgets.hovered.bg_stroke = egui::Stroke::NONE;

        ui.visuals_mut().widgets.active.bg_fill = hover_color;
        ui.visuals_mut().widgets.active.fg_stroke = fg_stroke;
        ui.visuals_mut().widgets.active.bg_stroke = egui::Stroke::NONE;

        ui.visuals_mut().override_text_color = Some(fg_color);

        egui::ComboBox::from_id_salt("sort_mode_secondary")
            .selected_text(match app.sort_mode {
                SortMode::Name => t!("secondary_toolbar.sort_name"),
                SortMode::Date => t!("secondary_toolbar.sort_date"),
                SortMode::Size => t!("secondary_toolbar.sort_size"),
                SortMode::Type => t!("secondary_toolbar.sort_type"),
                SortMode::DriveTotalSpace => t!("secondary_toolbar.sort_total_space"),
                SortMode::DriveFreeSpace => t!("secondary_toolbar.sort_free_space"),
                SortMode::DriveLetter => t!("secondary_toolbar.sort_drive_letter"),
            })
            .show_ui(ui, |ui| {
                if ui
                    .selectable_value(
                        &mut SortMode::Name,
                        app.sort_mode,
                        t!("secondary_toolbar.sort_name"),
                    )
                    .clicked()
                {
                    app.sort_mode = SortMode::Name;
                    if app.navigation_state.is_computer_view {
                        app.sort_mode_computer = SortMode::Name;
                    } else {
                        app.sort_mode_normal = SortMode::Name;
                    }
                    app.sort_items();
                    app.save_preferences();
                }

                if app.navigation_state.is_computer_view {
                    if ui
                        .selectable_value(
                            &mut SortMode::DriveTotalSpace,
                            app.sort_mode,
                            t!("secondary_toolbar.sort_total_space"),
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
                            t!("secondary_toolbar.sort_free_space"),
                        )
                        .clicked()
                    {
                        app.sort_mode = SortMode::DriveFreeSpace;
                        app.sort_mode_computer = SortMode::DriveFreeSpace;
                        app.sort_items();
                        app.save_preferences();
                    }
                    if ui
                        .selectable_value(
                            &mut SortMode::DriveLetter,
                            app.sort_mode,
                            t!("secondary_toolbar.sort_drive_letter"),
                        )
                        .clicked()
                    {
                        app.sort_mode = SortMode::DriveLetter;
                        app.sort_mode_computer = SortMode::DriveLetter;
                        app.sort_items();
                        app.save_preferences();
                    }
                } else {
                    if ui
                        .selectable_value(
                            &mut SortMode::Date,
                            app.sort_mode,
                            t!("secondary_toolbar.sort_date"),
                        )
                        .clicked()
                    {
                        app.sort_mode = SortMode::Date;
                        app.sort_mode_normal = SortMode::Date;
                        app.sort_items();
                        app.save_preferences();
                    }
                    if ui
                        .selectable_value(
                            &mut SortMode::Size,
                            app.sort_mode,
                            t!("secondary_toolbar.sort_size"),
                        )
                        .clicked()
                    {
                        app.sort_mode = SortMode::Size;
                        app.sort_mode_normal = SortMode::Size;
                        app.sort_items();
                        app.save_preferences();
                    }
                    if ui
                        .selectable_value(
                            &mut SortMode::Type,
                            app.sort_mode,
                            t!("secondary_toolbar.sort_type"),
                        )
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

    render_folders_position_button(ui, app);
}

fn render_folders_position_button(ui: &mut egui::Ui, app: &mut ImageViewerApp) {
    let locked = app.current_folder_locked;
    let current_position = match app.folders_position {
        FoldersPosition::Last => FoldersPosition::Last,
        FoldersPosition::First | FoldersPosition::Mixed => FoldersPosition::First,
    };
    let next_position = match current_position {
        FoldersPosition::First => FoldersPosition::Last,
        FoldersPosition::Last => FoldersPosition::First,
        FoldersPosition::Mixed => FoldersPosition::First,
    };
    let arrow = if current_position == FoldersPosition::First {
        "↑"
    } else {
        "↓"
    };
    let tooltip = if current_position == FoldersPosition::First {
        t!("status_bar.folders_first_hint").to_string()
    } else {
        t!("status_bar.folders_last_hint").to_string()
    };

    ui.scope(|ui| {
        if locked {
            ui.disable();
        }

        let hover_color = if ui.visuals().dark_mode {
            theme::color_dark_hover()
        } else {
            theme::color_hover()
        };
        let button_size = egui::vec2(
            theme::ICON_SIZE_LG + theme::PADDING_SM * 2.0 + 12.0,
            theme::ICON_SIZE_LG + theme::PADDING_SM * 2.0,
        );
        let (rect, response) = ui.allocate_exact_size(button_size, egui::Sense::click());

        if response.hovered() {
            ui.painter()
                .rect_filled(rect, theme::PADDING_SM, hover_color);
        }

        let icon_color = if locked {
            [150, 150, 150, 180]
        } else if ui.visuals().dark_mode {
            [220, 220, 220, 255]
        } else {
            [60, 60, 60, 255]
        };
        let icon_tint = egui::Color32::from_rgba_premultiplied(
            icon_color[0],
            icon_color[1],
            icon_color[2],
            icon_color[3],
        );
        let folder_center = egui::pos2(
            rect.left() + theme::PADDING_SM + theme::ICON_SIZE_LG * 0.5,
            rect.center().y - 1.0,
        );

        if let Some(texture) = app.svg_icon_manager.get_icon(
            ui.ctx(),
            "folder",
            theme::ICON_SIZE_LG as u32,
            icon_color,
        ) {
            let icon_rect = egui::Rect::from_center_size(
                folder_center,
                egui::vec2(theme::ICON_SIZE_LG, theme::ICON_SIZE_LG),
            );
            ui.painter().image(
                texture.id(),
                icon_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        } else {
            ui.painter().text(
                folder_center,
                egui::Align2::CENTER_CENTER,
                "📁",
                egui::FontId::proportional(theme::ICON_SIZE_LG),
                icon_tint,
            );
        }

        ui.painter().text(
            egui::pos2(rect.right() - theme::PADDING_SM - 5.0, rect.center().y),
            egui::Align2::CENTER_CENTER,
            arrow,
            egui::FontId::proportional(18.0),
            icon_tint,
        );

        let response = response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(tooltip);

        if response.clicked() {
            app.folders_position = next_position;
            if !app.current_folder_locked {
                app.folders_position_normal = app.folders_position;
            }
            app.sort_items();
            app.save_preferences();
        }
    });
}
