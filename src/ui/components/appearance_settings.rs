use crate::app::navigation_state::ThemeMode;
use crate::ui::components::settings_ui;
use crate::ui::theme;
use eframe::egui::{self, RichText};
use rust_i18n::t;

pub fn render_appearance_page(ui: &mut egui::Ui, app: &mut crate::app::ImageViewerApp) -> bool {
    settings_ui::section_header(
        ui,
        &t!("settings.appearance"),
        &t!("settings.appearance_description"),
    );
    let changed = render_theme_settings(ui, &mut app.theme_mode);
    super::font_settings::render(ui);
    settings_ui::subsection_header(ui, &t!("settings.file_lists"), "");
    if settings_ui::toggle_row(
        ui,
        &t!("settings.alternating_row_colors"),
        &mut app.list_stripes_enabled,
    ) {
        crate::ui::list_stripes::remember(ui.ctx(), app.list_stripes_enabled);
        app.save_preferences();
        app.force_save_preferences();
    }
    let previous = app.folder_appearance;
    render_folder_appearance(ui, &mut app.folder_appearance);
    if previous != app.folder_appearance {
        app.apply_folder_appearance(ui.ctx());
        app.save_preferences();
        app.force_save_preferences();
    }
    changed
}

fn render_theme_settings(ui: &mut egui::Ui, theme_mode: &mut ThemeMode) -> bool {
    let mut changed = false;

    settings_ui::subsection_header(ui, &t!("settings.themes"), "");

    let modes: [(ThemeMode, &str); 5] = [
        (ThemeMode::Light, &t!("settings.theme_light")),
        (ThemeMode::Dark, &t!("settings.theme_dark")),
        (ThemeMode::DarkDeep, &t!("settings.theme_dark_deep")),
        (ThemeMode::GitHubDark, &t!("settings.theme_github_dark")),
        (ThemeMode::System, &t!("settings.theme_system")),
    ];
    let selected = modes
        .iter()
        .position(|(mode, _)| mode == theme_mode)
        .unwrap_or(0);
    let labels: Vec<&str> = modes.iter().map(|(_, name)| *name).collect();
    if let Some(idx) = settings_ui::choice_list(ui, &labels, selected) {
        *theme_mode = modes[idx].0;
        changed = true;
    }

    ui.add_space(12.0);
    ui.label(
        RichText::new(t!("settings.theme_apply_immediately"))
            .size(12.0)
            .color(theme::secondary_text_color(ui.ctx())),
    );

    changed
}

fn render_folder_appearance(
    ui: &mut egui::Ui,
    appearance: &mut crate::domain::folder_appearance::FolderAppearance,
) {
    use crate::domain::folder_appearance::FolderAppearance;
    settings_ui::subsection_header(ui, &t!("settings.folder_colors"), "");
    let labels = [
        t!("settings.folder_original"),
        t!("settings.folder_pastel_yellow"),
        t!("settings.folder_blue_gray"),
        t!("settings.folder_dark_slate_blue"),
        t!("settings.folder_lavender"),
    ];
    let labels: Vec<&str> = labels.iter().map(|label| label.as_ref()).collect();
    let selected = FolderAppearance::ALL
        .iter()
        .position(|value| value == appearance)
        .unwrap_or(0);
    if let Some(index) = settings_ui::choice_list(ui, &labels, selected) {
        *appearance = FolderAppearance::ALL[index];
    }
}
