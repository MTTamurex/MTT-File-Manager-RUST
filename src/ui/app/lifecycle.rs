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

    // Detect background→foreground transitions (window regains focus after being unfocused)
    // This catches the case where the app was NOT minimized but simply behind other windows,
    // which still causes OS paging and GPU wake spikes on return.
    let is_focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
    if is_focused && !app.was_focused {
        let idle_secs = app.last_restore_time.elapsed().as_secs_f64();
        if idle_secs > 5.0 {
            app.minimized_duration_secs = idle_secs;
            app.last_restore_time = std::time::Instant::now();
            // Reset inflated peak so budgets aren't starved by the wake spike
            app.frame_time_peak_ms = app.frame_time_avg_ms.max(16.0);
            log::info!(
                "[LIFECYCLE] App regained focus after {:.1}s in background - resetting peak metrics",
                idle_secs
            );
        }
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
            // Reset inflated peak so budgets aren't starved by the wake spike
            app.frame_time_peak_ms = app.frame_time_avg_ms.max(16.0);
            log::info!(
                "[LIFECYCLE] App restored after {:.1}s of inactivity - resetting peak metrics",
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
    // Stop the GC worker thread before tearing down.
    crate::app::init_workers::stop_gc_worker();

    // ── ROOT CAUSE ──────────────────────────────────────────────────────
    // A background thread is stuck in NtQueryAttributesFile (kernel mode)
    // waiting for the OneDrive cloud filter driver (cldflt.sys).
    //
    // CancelSynchronousIo ALSO blocks when the driver ignores the cancel
    // request, so we must fire it from a background thread and NOT wait.
    // ────────────────────────────────────────────────────────────────────

    // 1. Fire-and-forget: attempt to cancel stuck I/O from a background
    //    thread so it doesn't block the main thread / UI closure.
    let _ = std::thread::Builder::new()
        .name("io-cancel".into())
        .spawn(|| {
            cancel_all_pending_io();
        });

    // 2. Shut down libmpv to release GPU/decoder resources.
    if let Some(preview) = app.media_preview.as_mut() {
        preview.shutdown();
    }
    app.media_preview = None;

    // 3. Kill standalone video player process if running
    app.kill_video_player_process();

    // 4. Persist user preferences
    app.force_save_preferences();
    log::info!("[EXIT] Preferences saved. Terminating.");

    // 5. Force-kill immediately. The io-cancel thread may still be
    //    blocked in CancelSynchronousIo — that's fine, TerminateProcess
    //    will tear down the whole process (including that thread).
    unsafe {
        windows::Win32::System::Threading::TerminateProcess(
            windows::Win32::System::Threading::GetCurrentProcess(),
            0,
        )
        .ok();
    }
    std::process::abort();
}

/// Cancel all pending synchronous I/O on every thread in this process.
///
/// Enumerates threads via `CreateToolhelp32Snapshot`, opens each one, and
/// calls `CancelSynchronousIo`. This unblocks threads stuck in kernel‑mode
/// filesystem calls (e.g. `NtQueryAttributesFile` waiting on a minifilter
/// driver like OneDrive's `cldflt.sys`).
pub fn cancel_all_pending_io() {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD,
        THREADENTRY32,
    };
    use windows::Win32::System::Threading::{
        GetCurrentProcessId, GetCurrentThreadId, OpenThread, THREAD_TERMINATE,
    };
    use windows::Win32::System::IO::CancelSynchronousIo;

    let current_pid = unsafe { GetCurrentProcessId() };
    let current_tid = unsafe { GetCurrentThreadId() };

    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    let snapshot = match snapshot {
        Ok(h) => h,
        Err(_) => return,
    };

    let mut entry = THREADENTRY32 {
        dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
        ..Default::default()
    };

    let mut cancelled = 0u32;
    unsafe {
        if Thread32First(snapshot, &mut entry).is_ok() {
            loop {
                if entry.th32OwnerProcessID == current_pid
                    && entry.th32ThreadID != current_tid
                {
                    if let Ok(thread_handle) =
                        OpenThread(THREAD_TERMINATE, false, entry.th32ThreadID)
                    {
                        if CancelSynchronousIo(thread_handle).is_ok() {
                            cancelled += 1;
                        }
                        let _ = CloseHandle(thread_handle);
                    }
                }
                if Thread32Next(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
    }

    if cancelled > 0 {
        log::info!("[EXIT] Cancelled synchronous I/O on {} thread(s)", cancelled);
    }
}
