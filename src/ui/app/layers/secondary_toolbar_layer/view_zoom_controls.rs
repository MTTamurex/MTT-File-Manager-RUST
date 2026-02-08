use crate::app::ImageViewerApp;
use crate::domain::file_entry::ViewMode;
use crate::ui::theme;
use crate::ui::widgets;
use eframe::egui;

pub(super) fn render_view_and_zoom_controls(ui: &mut egui::Ui, app: &mut ImageViewerApp) {
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
}
