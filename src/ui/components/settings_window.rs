use crate::app::navigation_state::{SettingsSection, ThemeMode};
use crate::app::shortcuts::{ShortcutBindings, ShortcutEditorState};
use crate::ui::theme;
use eframe::egui::{self, Color32, Margin, RichText, Stroke};
use rust_i18n::t;

const BACKDROP_ALPHA: u8 = 72;

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
) -> SettingsWindowOutput {
    let mut keep_open = show_window;
    let mut language_changed = false;
    let mut theme_changed = false;
    let mut backend_changed = false;
    let mut shortcuts_changed = false;
    let mut recycle_bin_changed = false;
    let mut diagnostic_mode_changed = false;
    let mut open_diagnostic_folder = false;

    let screen_rect = ctx.screen_rect();

    // ── Backdrop (blocks interaction outside the modal) ──────────────────────
    let mut close_from_backdrop = false;
    egui::Area::new(egui::Id::from("settings_window_backdrop"))
        .fixed_pos(screen_rect.min)
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            ui.set_min_size(screen_rect.size());
            let backdrop_rect = ui.max_rect();
            let backdrop_resp = ui.interact(
                backdrop_rect,
                ui.id().with("settings_window_backdrop_interact"),
                egui::Sense::click(),
            );
            ui.painter().rect_filled(
                backdrop_rect,
                0.0,
                Color32::from_black_alpha(BACKDROP_ALPHA),
            );
            if backdrop_resp.clicked() {
                close_from_backdrop = true;
            }
        });

    if close_from_backdrop {
        keep_open = false;
    }

    // ESC closes
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        keep_open = false;
    }

    let dark_mode = ctx.style().visuals.dark_mode;
    let bg_color = if dark_mode {
        Color32::from_rgb(50, 50, 50)
    } else {
        Color32::from_rgb(250, 250, 250)
    };

    let frame = egui::Frame::new()
        .inner_margin(Margin {
            left: 16,
            right: 16,
            top: 8,
            bottom: 12,
        })
        .corner_radius(10.0)
        .fill(bg_color)
        .stroke(Stroke::new(
            1.0,
            if dark_mode {
                Color32::from_gray(70)
            } else {
                Color32::from_gray(220)
            },
        ))
        .shadow(egui::epaint::Shadow {
            spread: 4,
            blur: 12,
            color: Color32::from_black_alpha(25),
            offset: [0, 3],
        });

    egui::Window::new(t!("settings.window_title"))
        .id(egui::Id::new("settings_window"))
        .open(&mut keep_open)
        .collapsible(false)
        .resizable(true)
        .default_width(760.0)
        .default_height(480.0)
        .min_width(700.0)
        .min_height(420.0)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .frame(frame)
        .show(ctx, |ui| {
            ui.set_min_size(egui::vec2(700.0, 420.0));
            let content_height = ui.available_height();

            ui.horizontal_top(|ui| {
                let panel_height = content_height.max(300.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(180.0, panel_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| render_settings_sidebar(ui, active_section, dark_mode),
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
                                    ui.label(RichText::new(t!("settings.show_recycle_bin").to_string()).strong().color(theme::text_color(dark_mode)));
                                    ui.add_space(4.0);
                                    if ui.checkbox(show_recycle_bin, RichText::new(t!("settings.show_recycle_bin")).color(theme::text_color(dark_mode))).changed() {
                                        recycle_bin_changed = true;
                                    }
                                }
                                SettingsSection::Diagnostics => {
                                    ui.label(
                                        RichText::new(t!("settings.diagnostics").to_string())
                                            .size(16.0)
                                            .strong()
                                            .color(theme::text_color(dark_mode)),
                                    );
                                    ui.add_space(4.0);
                                    ui.label(RichText::new(t!("settings.diagnostics_description")).size(13.0).color(theme::secondary_text_color(dark_mode)));
                                    ui.add_space(8.0);
                                    if ui
                                        .checkbox(
                                            diagnostic_mode,
                                            RichText::new(t!("settings.diagnostics_enable")).color(theme::text_color(dark_mode)),
                                        )
                                        .changed()
                                    {
                                        diagnostic_mode_changed = true;
                                    }
                                    ui.add_space(6.0);
                                    if ui.button(t!("settings.diagnostics_open_folder")).clicked() {
                                        open_diagnostic_folder = true;
                                    }
                                    ui.add_space(12.0);
                                    ui.group(|ui| {
                                        ui.set_width(ui.available_width());
                                        ui.label(
                                            RichText::new(
                                                t!("settings.diagnostics_privacy_title").to_string(),
                                            )
                                            .strong()
                                            .color(theme::text_color(dark_mode)),
                                        );
                                        ui.add_space(4.0);
                                        ui.small(RichText::new(t!("settings.diagnostics_privacy_scope")).color(theme::secondary_text_color(dark_mode)));
                                        ui.add_space(4.0);
                                        ui.small(RichText::new(t!("settings.diagnostics_privacy_excludes")).color(theme::secondary_text_color(dark_mode)));
                                        ui.add_space(4.0);
                                        ui.small(RichText::new(t!("settings.diagnostics_privacy_transmission")).color(theme::secondary_text_color(dark_mode)));
                                    });
                                    ui.add_space(6.0);
                                    ui.small(RichText::new(t!("settings.diagnostics_note")).color(theme::secondary_text_color(dark_mode)));
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

fn render_settings_sidebar(ui: &mut egui::Ui, active_section: &mut SettingsSection, dark_mode: bool) {
    ui.spacing_mut().item_spacing.y = 8.0;
    ui.label(
        RichText::new(t!("settings.categories").to_string())
            .strong()
            .color(theme::text_color(dark_mode)),
    );
    ui.add_space(4.0);

    ui.selectable_value(
        active_section,
        SettingsSection::General,
        RichText::new(t!("settings.general")).color(theme::text_color(dark_mode)),
    );
    ui.selectable_value(
        active_section,
        SettingsSection::Diagnostics,
        RichText::new(t!("settings.diagnostics")).color(theme::text_color(dark_mode)),
    );
    ui.selectable_value(
        active_section,
        SettingsSection::Shortcuts,
        RichText::new(t!("settings.shortcuts")).color(theme::text_color(dark_mode)),
    );
    ui.selectable_value(
        active_section,
        SettingsSection::VirtualDrives,
        RichText::new(t!("settings.virtual_drives")).color(theme::text_color(dark_mode)),
    );
    ui.selectable_value(
        active_section,
        SettingsSection::About,
        RichText::new(t!("settings.about")).color(theme::text_color(dark_mode)),
    );
}
