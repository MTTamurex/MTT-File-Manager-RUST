use crate::app::ImageViewerApp;
use crate::infrastructure::onedrive;
use eframe::egui;

pub fn handle_startup_sequence(app: &mut ImageViewerApp, ctx: &egui::Context) {
    if app.startup_tick < 5 {
        app.startup_tick += 1;

        if app.startup_tick == 1 {
            // Frame 1: Apply saved geometry while window is hidden
            if app.layout.saved_is_maximized {
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::Vec2::new(
                    app.layout.saved_window_width,
                    app.layout.saved_window_height,
                )));
            }
        }

        if app.startup_tick == 5 {
            // Frame 5: Reveal the window
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);

            // Remove pinned folders that were deleted while the app was closed
            app.cleanup_deleted_pinned_folders();

            // FINAL INITIALIZATION: Now that the UI is ready, ensure the initial tab is populated
            if app.navigation_state.is_computer_view {
                app.setup_computer_view();
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
        freeze_layout, layout_phase, WindowLayoutPhase,
    };

    let (size_changed, maximized_changed, fullscreen_changed, is_minimized, minimized_changed) = ctx.input(|i| {
        let mut size_changed = false;
        let mut maximized_changed = false;

        // Detect if window is minimized
        let minimized = i.viewport().minimized.unwrap_or(false);
        let prev_minimized = app.layout.saved_is_minimized;
        let minimized_changed = minimized != prev_minimized;

        if let Some(rect) = i.viewport().inner_rect {
            // Only save size when NOT maximized
            if !i.viewport().maximized.unwrap_or(false) {
                if (app.layout.saved_window_width - rect.width()).abs() > 1.0
                    || (app.layout.saved_window_height - rect.height()).abs() > 1.0
                {
                    size_changed = true;
                }
                app.layout.saved_window_width = rect.width();
                app.layout.saved_window_height = rect.height();
            }
        }

        let new_maximized = i.viewport().maximized.unwrap_or(false);
        if new_maximized != app.layout.saved_is_maximized {
            maximized_changed = true;
        }
        app.layout.saved_is_maximized = new_maximized;

        let new_fullscreen = i.viewport().fullscreen.unwrap_or(false);
        let fullscreen_changed = new_fullscreen != app.layout.saved_is_fullscreen;
        app.layout.saved_is_fullscreen = new_fullscreen;

        (
            size_changed,
            maximized_changed,
            fullscreen_changed,
            minimized,
            minimized_changed,
        )
    });

    // Handle minimization state changes - CRITICAL for OneDrive thread management
    if minimized_changed {
        app.layout.saved_is_minimized = is_minimized;
        onedrive::set_app_minimized(is_minimized);

        if is_minimized {
            log::info!("[LIFECYCLE] App minimized - canceling OneDrive operations");
            // Track when we were minimized to calculate inactivity duration on restore
            app.last_restore_time = std::time::Instant::now();
        } else {
            // Calculate how long the app was minimized
            let minimized_secs = app.last_restore_time.elapsed().as_secs_f64();
            app.minimized_duration_secs = minimized_secs;
            app.last_restore_time = std::time::Instant::now();
            log::info!(
                "[LIFECYCLE] App restored after {:.1}s of inactivity - throttling operations",
                minimized_secs
            );
        }
    }

    // LAYOUT FREEZE: Capture sidebar widths before minimize
    // This happens when egui reports minimized but we haven't frozen yet
    if is_minimized && layout_phase() == WindowLayoutPhase::Normal {
        // Freeze layout with current sidebar widths
        freeze_layout(app.layout.sidebar_left_width, app.layout.sidebar_right_width);
    }

    // Save preferences when window state changes
    if size_changed || maximized_changed {
        app.save_preferences();
    }

    if maximized_changed || fullscreen_changed {
        if let Some(hwnd) = app.native_hwnd {
            let no_round = app.layout.saved_is_maximized || app.layout.saved_is_fullscreen;
            crate::infrastructure::windows::window_corners::apply_window_corner_preference(
                hwnd,
                no_round,
            );
        }
    }
}

pub fn handle_exit(app: &mut ImageViewerApp) {
    // Kill standalone video player process if running
    app.kill_video_player_process();

    // Gracefully stop thumbnail workers waiting on the queue.
    app.thumbnail_queue.shutdown();

    // H-6: Drop all background-worker Senders so threads exit via RecvError.
    app.shutdown_background_workers();

    // Force save sidebar widths before exit (bypass debounce)
    app.force_save_preferences();
    log::info!(
        "[EXIT] Saved sidebar widths: L={}, R={}",
        app.layout.sidebar_left_width, app.layout.sidebar_right_width
    );
}
