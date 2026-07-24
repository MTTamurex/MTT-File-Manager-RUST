use crate::app::ImageViewerApp;
use crate::domain::file_entry::ViewMode;
use crate::ui::theme;
use crate::ui::widgets;
use eframe::egui;
use rust_i18n::t;

pub(super) fn render_view_and_zoom_controls(ui: &mut egui::Ui, app: &mut ImageViewerApp) {
    let locked = app.current_folder_locked;
    let mut view_mode_changed = false;

    ui.scope(|ui| {
        if locked {
            ui.disable();
        }
        let svg_manager = &mut app.svg_icon_manager;
        if widgets::toggle_icon_button(
            ui,
            svg_manager,
            theme::ICON_VIEW_DETAILS,
            matches!(app.view_mode, ViewMode::List),
            &t!("secondary_toolbar.details"),
        )
        .clicked()
            && !matches!(app.view_mode, ViewMode::List)
        {
            app.view_mode = ViewMode::List;
            if !locked {
                app.view_mode_normal = ViewMode::List;
            }
            view_mode_changed = true;
        }

        if widgets::toggle_icon_button(
            ui,
            svg_manager,
            theme::ICON_VIEW_COLUMNS,
            matches!(app.view_mode, ViewMode::ColumnList),
            &t!("secondary_toolbar.column_list"),
        )
        .clicked()
            && !matches!(app.view_mode, ViewMode::ColumnList)
        {
            app.view_mode = ViewMode::ColumnList;
            if !locked {
                app.view_mode_normal = ViewMode::ColumnList;
            }
            view_mode_changed = true;
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
            view_mode_changed = true;
        }

        if widgets::toggle_icon_button(
            ui,
            svg_manager,
            theme::ICON_VIEW_MILLER,
            matches!(app.view_mode, ViewMode::Miller),
            &t!("secondary_toolbar.miller_columns"),
        )
        .clicked()
            && !matches!(app.view_mode, ViewMode::Miller)
        {
            app.view_mode = ViewMode::Miller;
            if !locked {
                app.view_mode_normal = ViewMode::Miller;
            }
            view_mode_changed = true;
        }
    });

    if view_mode_changed {
        app.watch_current_folder();
        let selection_is_outside_current_folder = app
            .selected_file
            .as_ref()
            .is_some_and(|selected| !app.items.iter().any(|item| item.path == selected.path));
        if !matches!(app.view_mode, ViewMode::Miller) && selection_is_outside_current_folder {
            app.clear_file_view_selection();
        }
        if matches!(
            app.view_mode,
            ViewMode::Grid | ViewMode::ColumnList | ViewMode::Miller
        ) {
            app.folder_size_state.cancel_batch();
        }
        app.save_preferences();
    }

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
            "dual_panel",
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
