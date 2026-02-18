use crate::app::ImageViewerApp;
use crate::domain::file_entry::SortMode;
use crate::ui::theme;
use eframe::egui;

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
                egui::RichText::new(sort_symbol).color(egui::Color32::BLACK),
            ))
            .on_hover_text("Inverter Ordem")
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
}
