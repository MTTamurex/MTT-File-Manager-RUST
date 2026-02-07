use eframe::egui;
use crate::app::ImageViewerApp;
use crate::infrastructure::onedrive;

pub fn handle_startup_sequence(app: &mut ImageViewerApp, ctx: &egui::Context) {
    if app.startup_tick < 5 {
        app.startup_tick += 1;

        if app.startup_tick == 1 {
            // Frame 1: Apply saved geometry while window is hidden
            if app.saved_is_maximized {
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::Vec2::new(
                    app.saved_window_width,
                    app.saved_window_height,
                )));
            }
        }

        if app.startup_tick == 5 {
            // Frame 5: Reveal the window
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);

            // FINAL INITIALIZATION: Agora que a UI está pronta, garante que a aba inicial está populada
            if app.is_computer_view {
                app.setup_computer_view();
                eprintln!("[COMPUTER-VIEW] startup: after setup, items.len()={}, all_items.len()={}", app.items.len(), app.all_items.len());
            } else {
                app.load_folder(false);
            }
            app.sync_to_tab();
        }

        // Keep the loop running fast during startup
        ctx.request_repaint();
    }
}

pub fn track_window_state(app: &mut ImageViewerApp, ctx: &egui::Context) {
    use crate::infrastructure::windows::window_subclass::{
        freeze_layout, layout_phase, WindowLayoutPhase
    };

    let (size_changed, maximized_changed, is_minimized, minimized_changed) = ctx.input(|i| {
        let mut size_changed = false;
        let mut maximized_changed = false;

        // Detect if window is minimized
        let minimized = i.viewport().minimized.unwrap_or(false);
        let prev_minimized = app.saved_is_minimized;
        let minimized_changed = minimized != prev_minimized;

        if let Some(rect) = i.viewport().inner_rect {
            // Only save size when NOT maximized
            if !i.viewport().maximized.unwrap_or(false) {
                if (app.saved_window_width - rect.width()).abs() > 1.0
                    || (app.saved_window_height - rect.height()).abs() > 1.0
                {
                    size_changed = true;
                }
                app.saved_window_width = rect.width();
                app.saved_window_height = rect.height();
            }
        }

        let new_maximized = i.viewport().maximized.unwrap_or(false);
        if new_maximized != app.saved_is_maximized {
            maximized_changed = true;
        }
        app.saved_is_maximized = new_maximized;

        (size_changed, maximized_changed, minimized, minimized_changed)
    });

    // Handle minimization state changes - CRITICAL for OneDrive thread management
    if minimized_changed {
        app.saved_is_minimized = is_minimized;
        onedrive::set_app_minimized(is_minimized);

        if is_minimized {
            eprintln!("[LIFECYCLE] App minimized - canceling OneDrive operations");
            // Track when we were minimized to calculate inactivity duration on restore
            app.last_restore_time = std::time::Instant::now();
        } else {
            // Calculate how long the app was minimized
            let minimized_secs = app.last_restore_time.elapsed().as_secs_f64();
            app.minimized_duration_secs = minimized_secs;
            app.last_restore_time = std::time::Instant::now();
            eprintln!(
                "[LIFECYCLE] App restored after {:.1}s of inactivity - throttling operations",
                minimized_secs
            );
        }
    }

    // LAYOUT FREEZE: Capture sidebar widths before minimize
    // This happens when egui reports minimized but we haven't frozen yet
    if is_minimized && layout_phase() == WindowLayoutPhase::Normal {
        // Freeze layout with current sidebar widths
        freeze_layout(app.sidebar_left_width, app.sidebar_right_width);
    }

    // Save preferences when window state changes
    if size_changed || maximized_changed {
        app.save_preferences();
    }
}

pub fn handle_exit(app: &mut ImageViewerApp) {
    // Force save sidebar widths before exit (bypass debounce)
    app.force_save_preferences();
    eprintln!(
        "[EXIT] Saved sidebar widths: L={}, R={}",
        app.sidebar_left_width, app.sidebar_right_width
    );
}
