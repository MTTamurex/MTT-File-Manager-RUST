use crate::app::navigation_state::{SettingsSection, ThemeMode};
use crate::app::shortcuts::{ShortcutBindings, ShortcutEditorState};
use eframe::egui;
use rust_i18n::t;
use std::path::Path;

pub struct SettingsWindowOutput {
    pub keep_open: bool,
    pub language_changed: bool,
    pub theme_changed: bool,
    pub backend_changed: bool,
    pub shortcuts_changed: bool,
    pub recycle_bin_changed: bool,
    pub diagnostic_mode_changed: bool,
    pub open_diagnostic_folder: bool,
}

pub fn render_settings_window(
    ctx: &egui::Context,
    show_window: bool,
    active_section: &mut SettingsSection,
    theme_mode: &mut ThemeMode,
    active_gpu_backend: &str,
    gpu_backend_preference: &mut String,
    shortcuts: &mut ShortcutBindings,
    shortcut_editor: &mut ShortcutEditorState,
    show_recycle_bin: &mut bool,
    diagnostic_mode: &mut bool,
    diagnostic_log_path: &Path,
) -> SettingsWindowOutput {
    let mut keep_open = show_window;
    let mut language_changed = false;
    let mut theme_changed = false;
    let mut backend_changed = false;
    let mut shortcuts_changed = false;
    let mut recycle_bin_changed = false;
    let mut diagnostic_mode_changed = false;
    let mut open_diagnostic_folder = false;

    egui::Window::new(t!("settings.window_title"))
        .id(egui::Id::new("settings_window"))
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

                if *active_section != SettingsSection::Shortcuts && shortcut_editor.is_capturing() {
                    shortcut_editor.clear();
                }

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
                                    language_changed |= crate::ui::components::language_settings::render_language_settings_section(ui);
                                    ui.add_space(16.0);
                                    theme_changed |= crate::ui::components::appearance_settings::render_appearance_settings_section(ui, theme_mode);
                                    ui.add_space(16.0);
                                    backend_changed |= crate::ui::components::backend_settings::render_backend_settings_section(ui, active_gpu_backend, gpu_backend_preference);
                                    ui.add_space(16.0);
                                    ui.label(egui::RichText::new(t!("settings.show_recycle_bin").to_string()).strong());
                                    ui.add_space(4.0);
                                    if ui.checkbox(show_recycle_bin, t!("settings.show_recycle_bin")).changed() {
                                        recycle_bin_changed = true;
                                    }
                                }
                                SettingsSection::Diagnostics => {
                                    ui.add_space(16.0);
                                    ui.label(
                                        egui::RichText::new(t!("settings.diagnostics").to_string())
                                            .strong(),
                                    );
                                    ui.add_space(4.0);
                                    ui.label(t!("settings.diagnostics_description"));
                                    ui.add_space(8.0);
                                    if ui
                                        .checkbox(
                                            diagnostic_mode,
                                            t!("settings.diagnostics_enable"),
                                        )
                                        .changed()
                                    {
                                        diagnostic_mode_changed = true;
                                    }
                                    ui.add_space(8.0);
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(
                                                diagnostic_log_path.display().to_string(),
                                            )
                                            .monospace(),
                                        )
                                        .wrap(),
                                    );
                                    ui.add_space(6.0);
                                    if ui.button(t!("settings.diagnostics_open_folder")).clicked() {
                                        open_diagnostic_folder = true;
                                    }
                                    ui.add_space(6.0);
                                    ui.small(t!("settings.diagnostics_note"));
                                }
                                SettingsSection::Shortcuts => {
                                    shortcuts_changed |= crate::ui::components::shortcut_settings::render_shortcut_settings_section(
                                        ui,
                                        shortcuts,
                                        shortcut_editor,
                                    );
                                }
                                SettingsSection::VirtualDrives => {
                                    crate::ui::components::virtual_drive_settings::render_virtual_drive_settings_section(ui);
                                }
                                SettingsSection::About => {
                                    crate::ui::components::about_settings::render_about_settings_section(ui);
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
        backend_changed,
        shortcuts_changed,
        recycle_bin_changed,
        diagnostic_mode_changed,
        open_diagnostic_folder,
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
        SettingsSection::Diagnostics,
        &*t!("settings.diagnostics"),
    );
    ui.selectable_value(
        active_section,
        SettingsSection::Shortcuts,
        &*t!("settings.shortcuts"),
    );
    ui.selectable_value(
        active_section,
        SettingsSection::VirtualDrives,
        &*t!("settings.virtual_drives"),
    );
    ui.selectable_value(
        active_section,
        SettingsSection::About,
        &*t!("settings.about"),
    );
}
