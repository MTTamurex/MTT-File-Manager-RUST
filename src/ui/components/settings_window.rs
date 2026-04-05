use crate::app::navigation_state::{SettingsSection, ThemeMode};
use eframe::egui;
use rust_i18n::t;

pub struct SettingsWindowOutput {
    pub keep_open: bool,
    pub language_changed: bool,
    pub theme_changed: bool,
}

pub fn render_settings_window(
    ctx: &egui::Context,
    show_window: bool,
    active_section: &mut SettingsSection,
    theme_mode: &mut ThemeMode,
) -> SettingsWindowOutput {
    let mut keep_open = show_window;
    let mut language_changed = false;
    let mut theme_changed = false;

    egui::Window::new(t!("settings.window_title"))
        .open(&mut keep_open)
        .collapsible(false)
        .resizable(true)
        .default_width(760.0)
        .default_height(480.0)
        .min_width(700.0)
        .min_height(420.0)
        .show(ctx, |ui| {
            ui.set_min_size(egui::vec2(700.0, 420.0));
            let content_height = ui.available_height();

            ui.horizontal_top(|ui| {
                let panel_height = content_height.max(300.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(180.0, panel_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| render_settings_sidebar(ui, active_section),
                );

                ui.separator();

                let content_size = egui::vec2(ui.available_width(), panel_height);
                ui.allocate_ui_with_layout(
                    content_size,
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("settings_window_content")
                            .auto_shrink([false, false])
                            .show(ui, |ui| match *active_section {
                                SettingsSection::General => {
                                    theme_changed |= crate::ui::components::appearance_settings::render_appearance_settings_section(ui, theme_mode);
                                    ui.add_space(16.0);
                                    language_changed |= crate::ui::components::language_settings::render_language_settings_section(ui);
                                }
                                SettingsSection::VirtualDrives => {
                                    crate::ui::components::virtual_drive_settings::render_virtual_drive_settings_section(ui);
                                }
                            });
                    },
                );
            });
        });

    SettingsWindowOutput {
        keep_open,
        language_changed,
        theme_changed,
    }
}

fn render_settings_sidebar(ui: &mut egui::Ui, active_section: &mut SettingsSection) {
    ui.spacing_mut().item_spacing.y = 8.0;
    ui.label(egui::RichText::new(t!("settings.categories").to_string()).strong());
    ui.add_space(4.0);

    ui.selectable_value(
        active_section,
        SettingsSection::General,
        &*t!("settings.general"),
    );
    ui.selectable_value(
        active_section,
        SettingsSection::VirtualDrives,
        &*t!("settings.virtual_drives"),
    );
}