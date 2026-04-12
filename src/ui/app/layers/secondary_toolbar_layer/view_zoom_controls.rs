use crate::app::ImageViewerApp;
use crate::domain::file_entry::ViewMode;
use crate::ui::theme;
use crate::ui::widgets;
use eframe::egui;
use rust_i18n::t;

pub(super) fn render_view_and_zoom_controls(ui: &mut egui::Ui, app: &mut ImageViewerApp) {
    let locked = app.current_folder_locked;

    ui.scope(|ui| {
        if locked {
            ui.disable();
        }
        let svg_manager = &mut app.svg_icon_manager;
        if widgets::toggle_icon_button(
            ui,
            svg_manager,
            theme::ICON_LIST,
            matches!(app.view_mode, ViewMode::List),
            &t!("secondary_toolbar.list"),
        )
        .clicked()
            && !matches!(app.view_mode, ViewMode::List)
        {
            app.view_mode = ViewMode::List;
            if !locked {
                app.view_mode_normal = ViewMode::List;
            }
        }

        if widgets::toggle_icon_button(
            ui,
            svg_manager,
            theme::ICON_GRID,
            matches!(app.view_mode, ViewMode::Grid),
            &t!("secondary_toolbar.grid"),
        )
        .clicked()
            && !matches!(app.view_mode, ViewMode::Grid)
        {
            app.view_mode = ViewMode::Grid;
            if !locked {
                app.view_mode_normal = ViewMode::Grid;
            }
            // Cancel batch folder-size work — Grid mode only
            // calculates size on selection, not for all items.
            app.folder_size_state.cancel_batch();
        }
    });

    ui.separator();

    if widgets::toggle_icon_button(
        ui,
        &mut app.svg_icon_manager,
        "search_computer",
        app.global_search.active,
        &t!("secondary_toolbar.global_search"),
    )
    .clicked()
    {
        app.toggle_global_search();
    }

    ui.separator();

    ui.scope(|ui| {
        if matches!(app.view_mode, ViewMode::List) {
            ui.disable();
        }
        ui.add_sized(
            egui::vec2(80.0, 20.0),
            egui::Slider::new(&mut app.thumbnail_size, crate::ui::theme::THUMBNAIL_MIN..=256.0)
                .show_value(false),
        );
        ui.label(&*t!("secondary_toolbar.zoom"));
    });
}
