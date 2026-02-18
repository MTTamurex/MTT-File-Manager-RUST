use crate::app::ImageViewerApp;
use crate::ui::widgets;
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
                // Added ~36px for the lock button + separator
                let content_width =
                    6.0 * 28.0 + 30.0 + 110.0 + 28.0 + 2.0 * 28.0 + 80.0 + 80.0 + 4.0 * 8.0 + 16.0 * 12.0;
                let available = ui.available_width();
                let left_pad = ((available - content_width) / 2.0).max(0.0);
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
                "Bloqueio indisponível nesta vista",
            );
        });
        return;
    }

    let icon_name = if is_locked { "lock" } else { "lock_open" };
    let tooltip = if is_locked {
        "Vista bloqueada — clique para desbloquear"
    } else {
        "Bloquear vista desta pasta"
    };

    if widgets::toggle_icon_button(ui, &mut app.svg_icon_manager, icon_name, is_locked, tooltip)
        .clicked()
    {
        app.toggle_folder_lock();
        if app.current_folder_locked {
            app.filter_items();
            app.sort_items();
        }
    }
}
