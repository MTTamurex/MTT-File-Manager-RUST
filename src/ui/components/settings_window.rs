use crate::app::navigation_state::SettingsSection;
use crate::app::ImageViewerApp;
use crate::ui::components::settings_ui;
use crate::ui::theme;
use eframe::egui::{self, Color32, Margin, RichText, Stroke};
use rust_i18n::t;

const BACKDROP_ALPHA: u8 = 72;
const TITLE_BAR_HEIGHT: f32 = 48.0;
const SIDEBAR_WIDTH: f32 = 200.0;

pub struct SettingsWindowOutput {
    pub keep_open: bool,
    pub language_changed: bool,
    pub theme_changed: bool,
    pub backend_changed: bool,
    pub shortcuts_changed: bool,
    pub quick_access_changed: bool,
    pub recycle_bin_changed: bool,
    pub tags_changed: bool,
    pub diagnostic_mode_changed: bool,
    pub open_diagnostic_folder: bool,
}

/// Render the modal backdrop BEFORE panels to block their input.
/// Returns true if the backdrop was clicked (should close the window).
pub fn render_settings_backdrop(ctx: &egui::Context) -> bool {
    let screen_rect = ctx.viewport_rect();
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
    close_from_backdrop
}

pub fn render_settings_window(
    ctx: &egui::Context,
    app: &mut ImageViewerApp,
    close_from_backdrop: bool,
) -> SettingsWindowOutput {
    let mut keep_open = app.navigation_state.show_settings_window;
    let mut language_changed = false;
    let mut theme_changed = false;
    let mut backend_changed = false;
    let mut shortcuts_changed = false;
    let mut quick_access_changed = false;
    let mut recycle_bin_changed = false;
    let mut tags_changed = false;
    let mut diagnostic_mode_changed = false;
    let mut open_diagnostic_folder = false;

    if close_from_backdrop {
        keep_open = false;
    }

    // ESC closes
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        keep_open = false;
    }

    let dark_mode = ctx.global_style().visuals.dark_mode;
    let bg_color = if dark_mode {
        Color32::from_rgb(43, 43, 43)
    } else {
        Color32::from_rgb(246, 246, 246)
    };

    let frame = egui::Frame::new()
        .inner_margin(Margin::same(0))
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
            spread: 8,
            blur: 24,
            color: Color32::from_black_alpha(70),
            offset: [0, 8],
        });

    // Keep resize cursors and interaction without egui's highlighted edge.
    let original_style = ctx.global_style();
    let mut window_style = (*original_style).clone();
    window_style.visuals.widgets.hovered.bg_stroke = Stroke::NONE;
    window_style.visuals.widgets.active.bg_stroke = Stroke::NONE;
    ctx.set_global_style(window_style);

    egui::Window::new(t!("settings.window_title"))
        .id(egui::Id::new("settings_window"))
        .title_bar(false)
        .collapsible(false)
        .resizable(true)
        .default_width(760.0)
        .default_height(480.0)
        .min_width(700.0)
        .min_height(420.0)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .order(egui::Order::Foreground)
        .frame(frame)
        .show(ctx, |ui| {
            ui.visuals_mut().widgets.hovered.bg_stroke =
                original_style.visuals.widgets.hovered.bg_stroke;
            ui.visuals_mut().widgets.active.bg_stroke =
                original_style.visuals.widgets.active.bg_stroke;
            ui.set_min_size(egui::vec2(700.0, 420.0));
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 0.0);

            render_settings_title_bar(ui, app, dark_mode, &mut keep_open);

            ui.horizontal_top(|ui| {
                let panel_height = ui.available_height();
                ui.allocate_ui_with_layout(
                    egui::vec2(SIDEBAR_WIDTH, panel_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        egui::Frame::new()
                            .inner_margin(Margin {
                                left: 16,
                                right: 12,
                                top: 4,
                                bottom: 8,
                            })
                            .show(ui, |ui| {
                                render_settings_sidebar(
                                    ui,
                                    &mut app.navigation_state.active_settings_section,
                                )
                            });
                    },
                );

                if app.navigation_state.active_settings_section != SettingsSection::Shortcuts
                    && app.shortcut_editor.is_capturing()
                {
                    app.shortcut_editor.clear();
                }

                let (sep_rect, _) =
                    ui.allocate_exact_size(egui::vec2(1.0, panel_height), egui::Sense::hover());
                ui.painter().rect_filled(
                    sep_rect,
                    0.0,
                    if dark_mode {
                        Color32::from_gray(65)
                    } else {
                        Color32::from_gray(225)
                    },
                );

                let content_size = egui::vec2(ui.available_width(), panel_height);
                ui.allocate_ui_with_layout(
                    content_size,
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        egui::Frame::new()
                            .inner_margin(Margin {
                                left: 24,
                                right: 20,
                                top: 12,
                                bottom: 16,
                            })
                            .show(ui, |ui| {
                                let current_section = app.navigation_state.active_settings_section;
                                egui::ScrollArea::vertical()
                                    .id_salt("settings_window_content")
                                    .auto_shrink([false, false])
                                    .show(ui, |ui| {
                                        // Keep right-edge controls (switches) clear of the floating scrollbar.
                                        ui.set_max_width(ui.available_width() - 14.0);
                                        match current_section {
                                        SettingsSection::General => {
                                            language_changed |= crate::ui::components::language_settings::render_language_settings_section(ui);
                                            ui.add_space(20.0);
                                            theme_changed |= crate::ui::components::appearance_settings::render_appearance_settings_section(ui, &mut app.theme_mode);
                                            ui.add_space(20.0);
                                            if settings_ui::toggle_row(ui, &t!("settings.show_quick_access_sidebar"), &mut app.show_quick_access) {
                                                quick_access_changed = true;
                                            }
                                            ui.add_space(10.0);
                                            if settings_ui::toggle_row(ui, &t!("settings.show_recycle_bin"), &mut app.show_recycle_bin) {
                                                recycle_bin_changed = true;
                                            }
                                        }
                                        SettingsSection::Diagnostics => {
                                            settings_ui::section_header(ui, &t!("settings.diagnostics"), &t!("settings.diagnostics_description"));
                                            if settings_ui::toggle_row(ui, &t!("settings.diagnostics_enable"), &mut app.diagnostic_mode) {
                                                diagnostic_mode_changed = true;
                                            }
                                            ui.add_space(10.0);
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
                                                ui.label(RichText::new(t!("settings.diagnostics_privacy_scope")).color(theme::secondary_text_color(dark_mode)));
                                                ui.add_space(4.0);
                                                ui.label(RichText::new(t!("settings.diagnostics_privacy_excludes")).color(theme::secondary_text_color(dark_mode)));
                                                ui.add_space(4.0);
                                                ui.label(RichText::new(t!("settings.diagnostics_privacy_transmission")).color(theme::secondary_text_color(dark_mode)));
                                            });
                                            ui.add_space(6.0);
                                            ui.label(RichText::new(t!("settings.diagnostics_note")).color(theme::secondary_text_color(dark_mode)));
                                        }
                                        SettingsSection::Graphics => {
                                            backend_changed |= crate::ui::components::backend_settings::render_backend_settings_section(ui, &app.active_gpu_backend, &mut app.gpu_backend_preference);
                                        }
                                        SettingsSection::Shortcuts => {
                                            shortcuts_changed |= crate::ui::components::shortcut_settings::render_shortcut_settings_section(
                                                ui,
                                                &mut app.shortcuts,
                                                &mut app.shortcut_editor,
                                            );
                                        }
                                        SettingsSection::Tags => {
                                            let tag_output = crate::ui::components::tag_settings::render_tag_settings_section(ui, app);
                                            tags_changed |= tag_output.show_tags_changed;
                                        }
                                        SettingsSection::Organizer => {
                                            crate::ui::components::organizer_settings::render_organizer_settings_section(ui, app);
                                        }
                                        SettingsSection::VirtualDrives => {
                                            crate::ui::components::virtual_drive_settings::render_virtual_drive_settings_section(ui);
                                        }
                                        SettingsSection::About => {
                                            crate::ui::components::about_settings::render_about_settings_section(ui);
                                        }
                                        }
                                    });
                            });
                    },
                );
            });
        });

    ctx.set_global_style(original_style);

    SettingsWindowOutput {
        keep_open,
        language_changed,
        theme_changed,
        backend_changed,
        shortcuts_changed,
        quick_access_changed,
        recycle_bin_changed,
        tags_changed,
        diagnostic_mode_changed,
        open_diagnostic_folder,
    }
}

/// Custom title bar: settings icon + left-aligned title, modern close button.
fn render_settings_title_bar(
    ui: &mut egui::Ui,
    app: &mut ImageViewerApp,
    dark_mode: bool,
    keep_open: &mut bool,
) {
    let (header_rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), TITLE_BAR_HEIGHT),
        egui::Sense::hover(),
    );
    if !ui.is_rect_visible(header_rect) {
        return;
    }

    let icon_color = if dark_mode {
        [220, 220, 220, 255]
    } else {
        [60, 60, 60, 255]
    };
    if let Some(texture) = app
        .svg_icon_manager
        .get_icon(ui.ctx(), "settings", 40, icon_color)
    {
        let icon_rect = egui::Rect::from_center_size(
            egui::pos2(header_rect.min.x + 26.0, header_rect.center().y),
            egui::vec2(20.0, 20.0),
        );
        ui.painter().image(
            texture.id(),
            icon_rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }

    let title_rect = egui::Rect::from_min_max(
        egui::pos2(header_rect.min.x + 46.0, header_rect.center().y - 10.0),
        egui::pos2(header_rect.max.x - 52.0, header_rect.center().y + 10.0),
    );
    let title_galley = egui::WidgetText::from(
        RichText::new(t!("settings.window_title").to_string())
            .size(15.0)
            .strong()
            .color(theme::text_color(dark_mode)),
    )
    .into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::FontId::proportional(15.0),
    );
    let title_pos = egui::pos2(
        title_rect.min.x,
        header_rect.center().y - 0.5 * title_galley.size().y,
    );
    ui.painter()
        .galley(title_pos, title_galley, theme::text_color(dark_mode));

    let close_rect = egui::Rect::from_min_size(
        egui::pos2(header_rect.max.x - 44.0, header_rect.center().y - 14.0),
        egui::vec2(32.0, 28.0),
    );
    let close_resp = ui.interact(
        close_rect,
        ui.id().with("settings_close_button"),
        egui::Sense::click(),
    );
    let hovered = close_resp.hovered();
    ui.painter().rect_filled(
        close_rect,
        6.0,
        if hovered {
            Color32::from_rgb(196, 43, 28)
        } else {
            Color32::TRANSPARENT
        },
    );
    ui.painter().text(
        close_rect.center(),
        egui::Align2::CENTER_CENTER,
        "×",
        egui::FontId::proportional(16.0),
        if hovered {
            Color32::WHITE
        } else {
            theme::secondary_text_color(dark_mode)
        },
    );
    let close_resp = close_resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if close_resp.clicked() {
        *keep_open = false;
    }
}

fn render_settings_sidebar(ui: &mut egui::Ui, active_section: &mut SettingsSection) {
    ui.spacing_mut().item_spacing.y = 4.0;
    ui.add_space(8.0);

    if settings_ui::nav_item(
        ui,
        &t!("settings.general"),
        *active_section == SettingsSection::General,
    ) {
        *active_section = SettingsSection::General;
    }
    if settings_ui::nav_item(
        ui,
        &t!("settings.diagnostics"),
        *active_section == SettingsSection::Diagnostics,
    ) {
        *active_section = SettingsSection::Diagnostics;
    }
    if settings_ui::nav_item(
        ui,
        &t!("settings.graphics"),
        *active_section == SettingsSection::Graphics,
    ) {
        *active_section = SettingsSection::Graphics;
    }
    if settings_ui::nav_item(
        ui,
        &t!("settings.shortcuts"),
        *active_section == SettingsSection::Shortcuts,
    ) {
        *active_section = SettingsSection::Shortcuts;
    }
    if settings_ui::nav_item(
        ui,
        &t!("settings.tags"),
        *active_section == SettingsSection::Tags,
    ) {
        *active_section = SettingsSection::Tags;
    }
    if settings_ui::nav_item(
        ui,
        &t!("settings.organizer"),
        *active_section == SettingsSection::Organizer,
    ) {
        *active_section = SettingsSection::Organizer;
    }
    if settings_ui::nav_item(
        ui,
        &t!("settings.virtual_drives"),
        *active_section == SettingsSection::VirtualDrives,
    ) {
        *active_section = SettingsSection::VirtualDrives;
    }
    if settings_ui::nav_item(
        ui,
        &t!("settings.about"),
        *active_section == SettingsSection::About,
    ) {
        *active_section = SettingsSection::About;
    }
}
