use crate::app::ImageViewerApp;
use crate::infrastructure::onedrive;
use eframe::egui;

fn recover_empty_current_folder_after_restore(app: &mut ImageViewerApp, reason: &str) {
    if app.navigation_state.is_computer_view || app.navigation_state.is_recycle_bin_view {
        return;
    }
    if !app.search_query.is_empty() || app.is_loading_folder || app.items_rebuild_in_flight {
        return;
    }
    if !app.all_items.is_empty() {
        return;
    }

    log::warn!(
        "[LIFECYCLE] Empty current listing detected after restore ({reason}) - reloading: {}",
        app.navigation_state.current_path
    );
    app.loaded_path.clear();
    app.load_folder(false);
    app.ui_ctx.request_repaint();
}

pub fn handle_startup_sequence(app: &mut ImageViewerApp, ctx: &egui::Context) {
    if app.startup_tick < 5 {
        app.startup_tick += 1;

        if app.startup_tick == 1 {
            // Frame 1: Apply saved theme before anything else
            match app.theme_mode {
                crate::app::navigation_state::ThemeMode::Dark => {
                    ctx.set_visuals(egui::Visuals::dark())
                }
                crate::app::navigation_state::ThemeMode::Light => {
                    ctx.set_visuals(egui::Visuals::light())
                }
            }
            crate::ui::theme::apply_scroll_style(ctx);

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
        freeze_layout, layout_phase, try_unfreeze_layout, WindowLayoutPhase,
    };

    let (
        size_changed,
        maximized_changed,
        fullscreen_changed,
        is_minimized,
        minimized_changed,
        valid_inner_size,
    ) = ctx.input(|i| {
        let mut size_changed = false;
        let mut maximized_changed = false;

        // Detect if window is minimized
        let viewport = i.viewport();
        let minimized = viewport.minimized.unwrap_or(false);
        let prev_minimized = app.layout.saved_is_minimized;
        let minimized_changed = minimized != prev_minimized;
        let is_maximized = viewport.maximized.unwrap_or(false);
        let valid_inner_size = viewport.inner_rect.and_then(|rect| {
            let width = rect.width();
            let height = rect.height();
            (width.is_finite() && height.is_finite() && width > 100.0 && height > 100.0)
                .then(|| rect.size())
        });

        if let Some(size) = valid_inner_size {
            // Only save size when the window is visible, valid, and not maximized.
            if !minimized && !is_maximized {
                if (app.layout.saved_window_width - size.x).abs() > 1.0
                    || (app.layout.saved_window_height - size.y).abs() > 1.0
                {
                    size_changed = true;
                }
                app.layout.saved_window_width = size.x;
                app.layout.saved_window_height = size.y;
            }
        }

        if is_maximized != app.layout.saved_is_maximized {
            maximized_changed = true;
        }
        app.layout.saved_is_maximized = is_maximized;

        let new_fullscreen = viewport.fullscreen.unwrap_or(false);
        let fullscreen_changed = new_fullscreen != app.layout.saved_is_fullscreen;
        app.layout.saved_is_fullscreen = new_fullscreen;

        (
            size_changed,
            maximized_changed,
            fullscreen_changed,
            minimized,
            minimized_changed,
            valid_inner_size,
        )
    });

    if !is_minimized {
        if let Some(size) = valid_inner_size {
            try_unfreeze_layout(size.x, size.y);
        }
    }

    // Detect background→foreground transitions (window regains focus after being unfocused)
    // This catches the case where the app was NOT minimized but simply behind other windows,
    // which still causes OS paging and GPU wake spikes on return.
    let is_focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
    let mut restored_from_background = false;
    if is_focused && !app.was_focused {
        let idle_secs = app
            .focus_lost_at
            .map(|t| t.elapsed().as_secs_f64())
            .unwrap_or(0.0);
        app.focus_lost_at = None;
        if idle_secs > 5.0 {
            app.minimized_duration_secs = idle_secs;
            app.last_restore_time = std::time::Instant::now();
            // Hard-reset peak to the current average so adaptive throttling
            // doesn't starve upload budgets on the very first frames.
            app.frame_time_peak_ms = app.frame_time_avg_ms.max(16.0);
            // Only flush GPU textures after prolonged inactivity (≥60 s).
            // For shorter idle periods (10–59 s) the OS usually hasn't paged
            // out the GPU working set yet, so the existing TextureHandles are
            // still valid and clearing them just forces unnecessary re-uploads
            // that cause visible stutter.
            if idle_secs >= 60.0 {
                flush_gpu_textures_for_reupload(app, "focus-restore");
            }
            // Burst window: short and proportional.  The purpose is only to
            // let the upload pipeline run at full speed while the first few
            // frames are still slow from OS paging — not to keep burning
            // CPU/GPU on continuous repaints for many seconds after.
            let burst_secs = (2.0 + (idle_secs / 120.0)).min(5.0);
            app.restore_burst_until =
                Some(std::time::Instant::now() + std::time::Duration::from_secs_f64(burst_secs));
            log::info!(
                "[LIFECYCLE] App regained focus after {:.1}s in background - burst {:.1}s, texture_flush={}",
                idle_secs, burst_secs, idle_secs >= 60.0
            );
        }

        restored_from_background = true;
        app.request_current_folder_liveness_probe("window focus restored");
    }
    if !is_focused && app.was_focused {
        app.focus_lost_at = Some(std::time::Instant::now());
    }
    app.was_focused = is_focused;

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
            app.frame_time_peak_ms = app.frame_time_avg_ms.max(16.0);
            if minimized_secs >= 60.0 {
                flush_gpu_textures_for_reupload(app, "minimize-restore");
            }
            if minimized_secs > 5.0 {
                let burst_secs = (2.0 + minimized_secs / 120.0).min(5.0);
                app.restore_burst_until = Some(
                    std::time::Instant::now() + std::time::Duration::from_secs_f64(burst_secs),
                );
            }
            log::info!(
                "[LIFECYCLE] App restored after {:.1}s of inactivity - burst, texture_flush={}",
                minimized_secs,
                minimized_secs >= 60.0
            );

            restored_from_background = true;
            app.request_current_folder_liveness_probe("window restored from minimized");
        }
    }

    if restored_from_background {
        recover_empty_current_folder_after_restore(app, "background->foreground transition");
    }

    // LAYOUT FREEZE: Capture sidebar widths before minimize
    // This happens when egui reports minimized but we haven't frozen yet
    if is_minimized && (minimized_changed || layout_phase() == WindowLayoutPhase::Normal) {
        // Freeze layout with current sidebar widths
        freeze_layout(
            app.layout.sidebar_left_width,
            app.layout.sidebar_right_width,
        );
    }

    // Save preferences when window state changes
    if size_changed || maximized_changed {
        app.save_preferences();
    }

    if maximized_changed || fullscreen_changed {
        if let Some(hwnd) = app.native_hwnd {
            let no_round = app.layout.saved_is_maximized || app.layout.saved_is_fullscreen;
            crate::infrastructure::windows::window_corners::apply_window_corner_preference(
                hwnd, no_round,
            );
        }
    }
}

/// Flush GPU texture cache so visible items are re-uploaded from the RGBA RAM
/// cache on the next frame.  After prolonged inactivity the OS pages out the
/// GPU working set; keeping stale `TextureHandle`s causes slow first-paint
/// (page-faults on every draw call) or blank tiles.
///
/// Only the VRAM layer is cleared — the RGBA RAM cache (Layer 2) is kept intact,
/// so re-uploads are fast (no disk I/O).  Icons and folder previews are also
/// flushed since they suffer from the same paging effect.
fn flush_gpu_textures_for_reupload(app: &mut ImageViewerApp, reason: &str) {
    let textures = app.cache_manager.texture_cache.len();
    let folder_previews = app.cache_manager.folder_preview_cache.len();
    let icons = app.cache_manager.icon_cache.len();

    app.discard_thumbnail_pipeline_for_navigation(reason);
    app.cache_manager.texture_cache.clear();
    app.cache_manager.folder_preview_cache.clear();
    app.cache_manager.folder_preview_loading.clear();
    app.cache_manager.icon_cache.clear();
    app.cache_manager.loading_set.clear();
    app.cache_manager.pending_upload_set.clear();

    // Clear pending queue — stale generation data would be rejected anyway,
    // and new requests from the grid renderer will flow in immediately.
    app.pending_thumbnails.clear();

    log::info!(
        "[LIFECYCLE] Flushed thumbnail pipeline after prolonged inactivity reason={}: {} thumbnails, {} folder previews, {} icons",
        reason, textures, folder_previews, icons
    );
}

pub fn handle_exit(app: &mut ImageViewerApp) {
    // Stop the GC worker thread before tearing down.
    crate::app::init_workers::stop_gc_worker();

    // ── Phase 1: cooperative shutdown ────────────────────────────────────
    // Signal all background workers to exit by dropping their Senders.
    // Each worker loop breaks on RecvError and runs its destructors.
    app.shutdown_background_workers();
    log::info!("[EXIT] Background worker channels disconnected.");

    // Fire CancelSynchronousIo from a background thread to unblock any
    // threads stuck in kernel-mode I/O (e.g. OneDrive cldflt.sys).
    let _ = std::thread::Builder::new()
        .name("io-cancel".into())
        .spawn(|| {
            let cancelled =
                crate::infrastructure::windows::cancel_pending_io_on_current_process_threads();
            if cancelled > 0 {
                log::info!(
                    "[EXIT] Cancelled synchronous I/O on {} thread(s)",
                    cancelled
                );
            }
        });

    // Shut down libmpv to release GPU/decoder resources.
    if let Some(preview) = app.media_preview.as_mut() {
        preview.shutdown();
    }
    app.media_preview = None;

    // Kill standalone video player process if running
    app.kill_video_player_process();

    // Persist user preferences
    app.force_save_preferences();
    log::info!("[EXIT] Preferences saved.");

    // ── Phase 2: minimal grace for channel-drop propagation ───────────
    // Workers break on RecvError within microseconds.  A short yield is
    // sufficient; anything still alive is stuck in a kernel call and
    // process::exit will tear it down.
    std::thread::sleep(std::time::Duration::from_millis(30));
    log::info!("[EXIT] Grace period elapsed. Exiting.");
    crate::infrastructure::diagnostic_logger::flush();

    // ── Phase 3: exit process ────────────────────────────────────────────
    // std::process::exit runs libc atexit handlers (including SQLite's) and
    // is sufficient for clean teardown when workers have already stopped.
    // TerminateProcess is kept as a failsafe on a background thread in case
    // std::process::exit somehow hangs (e.g. a stuck atexit handler).
    let _ = std::thread::Builder::new()
        .name("exit-watchdog".into())
        .spawn(|| {
            std::thread::sleep(std::time::Duration::from_secs(1));
            log::error!("[EXIT] Clean exit hung — forcing TerminateProcess.");
            crate::infrastructure::windows::terminate_current_process(0);
        });

    std::process::exit(0);
}
