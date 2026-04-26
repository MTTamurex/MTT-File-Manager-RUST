use crate::app::ImageViewerApp;
use crate::ui::{theme, widgets};
use eframe::egui;

mod actions;
mod sort_controls;
mod view_zoom_controls;

pub(crate) fn render_secondary_toolbar_layer(app: &mut ImageViewerApp, ctx: &egui::Context) {
    let separator_color = if ctx.style().visuals.dark_mode {
        egui::Color32::from_rgb(80, 80, 80)
    } else {
        egui::Color32::from_rgb(210, 210, 210)
    };

    egui::TopBottomPanel::top("secondary_nav_bar")
        .show_separator_line(false)
        .exact_height(46.0)
        .frame(egui::Frame {
            fill: if ctx.style().visuals.dark_mode {
                egui::Color32::from_rgb(45, 45, 45)
            } else {
                egui::Color32::WHITE
            },
            inner_margin: egui::Margin {
                left: 8,
                right: 8,
                top: 7,
                bottom: 7,
            },
            ..Default::default()
        })
        .show(ctx, |ui| {
            let rect = ui.max_rect();
            ui.painter().hline(
                rect.x_range(),
                rect.bottom(),
                egui::Stroke::new(1.0, separator_color),
            );

            ui.horizontal(|ui| {
                let toggle_icon_size = theme::ICON_SIZE_SM - 2.0;
                let toggle_padding = 1.0;
                let toggle_button_width = toggle_icon_size + toggle_padding * 2.0;
                let toggle_gap = 12.0;
                let sidebar_tooltip = if app.show_left_sidebar {
                    rust_i18n::t!("secondary_toolbar.hide_sidebar")
                } else {
                    rust_i18n::t!("secondary_toolbar.show_sidebar")
                };

                if widgets::toggle_icon_button_sized(
                    ui,
                    &mut app.svg_icon_manager,
                    "sidebar_left_panel",
                    !app.show_left_sidebar,
                    &sidebar_tooltip,
                    toggle_icon_size,
                    toggle_padding,
                    1.0,
                )
                .clicked()
                {
                    app.show_left_sidebar = !app.show_left_sidebar;
                    app.tab_manager.active_mut().show_left_sidebar = app.show_left_sidebar;
                    app.save_preferences();
                }

                ui.add_space(toggle_gap);

                let action_button_count = if app.navigation_state.is_recycle_bin_view {
                    7.0
                } else {
                    6.0
                };
                let show_media_play_button = app.should_show_secondary_toolbar_media_play_button();
                let separator_count = if show_media_play_button { 5.0 } else { 4.0 };
                let total_item_count =
                    action_button_count + if show_media_play_button { 13.0 } else { 11.0 };
                let gap_count = total_item_count - 1.0;
                // Includes action buttons, sort controls, folder-position toggle,
                // lock, view buttons, the global-search button, and an optional
                // standalone media-player button for selected audio/video files.
                let content_width = action_button_count * 28.0
                    + 30.0
                    + 110.0
                    + 40.0
                    + 28.0
                    + 2.0 * 28.0
                    + 28.0
                    + if show_media_play_button { 28.0 } else { 0.0 }
                    + separator_count * 8.0
                    + gap_count * 12.0;
                let reserved_left_width = toggle_button_width + toggle_gap;
                let total_available = ui.available_width() + reserved_left_width;
                let left_pad =
                    ((total_available - content_width) / 2.0 - reserved_left_width).max(0.0);
                ui.add_space(left_pad);

                ui.spacing_mut().item_spacing = egui::vec2(12.0, 0.0);

                let action = actions::render_action_buttons(ui, app);
                ui.separator();
                sort_controls::render_sort_controls(ui, app);
                ui.separator();
                render_lock_button(ui, app);
                ui.separator();
                view_zoom_controls::render_view_and_zoom_controls(ui, app);
                actions::execute_action(action, app);
            });
        });
}

/// Render the folder lock toggle button (padlock icon).
fn render_lock_button(ui: &mut egui::Ui, app: &mut ImageViewerApp) {
    let is_locked = app.current_folder_locked;
    let is_special = app.navigation_state.is_computer_view
        || app.navigation_state.is_recycle_bin_view
        || app.navigation_state.current_path.is_empty();

    if is_special {
        // Disabled state — render a grayed-out lock icon with no interaction
        ui.scope(|ui| {
            ui.disable();
            let _ = widgets::toggle_icon_button(
                ui,
                &mut app.svg_icon_manager,
                "lock_open",
                false,
                &rust_i18n::t!("secondary_toolbar.lock_unavailable"),
            );
        });
        return;
    }

    let icon_name = if is_locked { "lock" } else { "lock_open" };
    let tooltip = if is_locked {
        rust_i18n::t!("secondary_toolbar.locked")
    } else {
        rust_i18n::t!("secondary_toolbar.lock_folder")
    };

    if widgets::toggle_icon_button(
        ui,
        &mut app.svg_icon_manager,
        icon_name,
        is_locked,
        &tooltip,
    )
    .clicked()
    {
        app.toggle_folder_lock();
        if app.current_folder_locked {
            app.filter_items();
            app.sort_items();
        }
    }
}
