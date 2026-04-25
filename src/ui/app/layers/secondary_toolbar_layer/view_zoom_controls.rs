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

    // Dual panel toggle button
    {
        let dual_active = app.dual_panel_enabled;
        let tooltip_text = if dual_active {
            t!("secondary_toolbar.dual_panel_disable")
        } else {
            t!("secondary_toolbar.dual_panel_enable")
        };
        if widgets::toggle_icon_button(
            ui,
            &mut app.svg_icon_manager,
            "sidebar_left_panel",
            dual_active,
            &tooltip_text,
        )
        .clicked()
        {
            app.dual_panel_toggle();
        }
    }

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

    if app.should_show_secondary_toolbar_media_play_button() {
        ui.separator();

        if widgets::icon_button(
            ui,
            &mut app.svg_icon_manager,
            "play",
            &t!("secondary_toolbar.open_selected_media_window"),
            None,
        )
        .clicked()
        {
            let _ = app.open_selected_media_in_standalone_player();
        }
    }
}
