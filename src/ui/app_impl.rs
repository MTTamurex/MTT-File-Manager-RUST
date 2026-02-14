use crate::app::ImageViewerApp;
use crate::infrastructure::windows::window_subclass::is_in_size_move;
use crate::ui::app;
use eframe::egui;

/// Periodic repaint interval to ensure drive bitmask checks run even when idle.
const DRIVE_BITMASK_REPAINT_MS: u64 = 3000;

impl eframe::App for ImageViewerApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let t_frame_start = std::time::Instant::now();

        // True while Windows is in interactive move/resize loop (WM_ENTERSIZEMOVE..EXITSIZEMOVE).
        let is_in_size_move = is_in_size_move();

        // 1. Initial validation
        if self.startup_tick == 0 {
            // Check for loaded fonts
            if let Some(rx) = &self.font_loader_rx {
                if let Ok(fonts) = rx.try_recv() {
                    ctx.set_fonts(fonts);
                    self.font_loader_rx = None; // Disable loader once done
                    ctx.request_repaint(); // Force refresh with new fonts
                }
            }

            // NOTE: Removed path.exists() check here because it can BLOCK indefinitely
            // on OneDrive cloud-only files, causing UI freeze. The file selection will
            // be cleared naturally when the user navigates away or refreshes the folder.
            // If we need this check, it should be done asynchronously in a worker thread.
        }

        // 2. Lifecycle: Startup sequence & window state tracking
        app::lifecycle::handle_startup_sequence(self, ctx);
        app::lifecycle::track_window_state(self, ctx);
        let frame_ms = ctx.input(|i| i.stable_dt) * 1000.0;

        if frame_ms > 0.0 {
            if self.frame_time_avg_ms <= 0.0 {
                self.frame_time_avg_ms = frame_ms;
            } else {
                self.frame_time_avg_ms = self.frame_time_avg_ms * 0.9 + frame_ms * 0.1;
            }
            if self.frame_time_peak_ms <= 0.0 {
                self.frame_time_peak_ms = frame_ms;
            } else {
                self.frame_time_peak_ms *= 0.95;
                if frame_ms > self.frame_time_peak_ms {
                    self.frame_time_peak_ms = frame_ms;
                }
            }
            self.fps_avg = if self.frame_time_avg_ms > 0.0 {
                1000.0 / self.frame_time_avg_ms
            } else {
                0.0
            };
        }

        // 3. Infrastructure updates (throttle heavy processing during interactive move/resize)
        self.ensure_window_handle(frame);
        if !is_in_size_move {
            // PERF TIMING: Detect slow frames after inactivity
            let t0 = std::time::Instant::now();
            self.process_incoming_messages(ctx);
            let t1 = std::time::Instant::now();
            self.refresh_drives_if_needed();
            // Ensure the bitmask check runs even when the app is idle (no mouse/keyboard).
            // Without this, egui won't repaint and the timer never fires.
            ctx.request_repaint_after(std::time::Duration::from_millis(DRIVE_BITMASK_REPAINT_MS));
            let t2 = std::time::Instant::now();
            self.poll_drive_scan();
            self.poll_drive_info();
            let t3 = std::time::Instant::now();
            // Flush debounced preferences (max once per second)
            self.flush_preferences_if_needed();
            let t4 = std::time::Instant::now();
            // Bound long-session cache growth without disrupting interactive work.
            self.run_memory_maintenance();
            let t5 = std::time::Instant::now();

            let msg_ms = t1.duration_since(t0).as_millis();
            let drives_ms = t2.duration_since(t1).as_millis();
            let poll_ms = t3.duration_since(t2).as_millis();
            let prefs_ms = t4.duration_since(t3).as_millis();
            let memory_ms = t5.duration_since(t4).as_millis();
            if msg_ms + drives_ms + poll_ms + prefs_ms + memory_ms > 50 {
                log::warn!(
                    "[PERF] Slow infrastructure: messages={}ms drives={}ms poll={}ms prefs={}ms memory={}ms",
                    msg_ms, drives_ms, poll_ms, prefs_ms, memory_ms
                );
            }
        }

        let t_icons_start = std::time::Instant::now();
        self.ensure_folder_icon(ctx);
        self.ensure_computer_icon(ctx);
        self.item_icon_loader.ensure_folder_icon(ctx);
        let t_icons_end = std::time::Instant::now();
        let icons_ms = t_icons_end.duration_since(t_icons_start).as_millis();
        if icons_ms > 50 {
            log::warn!("[PERF] Slow ensure_icons: {}ms", icons_ms);
        }

        // Poll background icon extractions (sidebar drive/folder icons)
        self.item_icon_loader.poll_async_icons(ctx);

        // 4. Input: Keyboard shortcuts (resize borders handled by native subclass)
        if !is_in_size_move {
            app::input::handle_input(self, ctx);
        }

        // 5. Layout: Status Bar (Bottom) - lightweight, always render
        app::layers::render_status_bar_layer(self, ctx);

        // 6. Layout: Tab Bar (Top 1) - lightweight, always render
        app::layers::render_tab_bar_layer(self, ctx, frame);

        // 7. Layout: Toolbar (Top 2) - lightweight, always render
        app::layers::render_toolbar_layer(self, ctx);

        // 7b. Layout: Secondary Toolbar (Top 3) - lightweight, always render
        app::layers::render_secondary_toolbar_layer(self, ctx);

        // 8. Layout: Main Panels (Sidebar, Preview, Central)
        // Keep full rendering even during move/resize so content/video stays visible and synchronized.
        let t_panels = std::time::Instant::now();
        app::panels::render_panels(self, ctx, frame);
        let panels_ms = t_panels.elapsed().as_millis();
        if panels_ms > 50 {
            log::warn!("[PERF] Slow render_panels: {}ms", panels_ms);
        }

        // 9. Operations: Context Menu (Rendering & Actions)
        app::menu_handler::handle_context_menu(self, ctx);

        // 10. Operations: Resize borders (on top) - REMOVED, handled by native subclass
        // app::input::handle_resize_borders(self, ctx);

        // 11. Virtual drive settings modal
        if self.navigation_state.show_virtual_drive_settings {
            self.navigation_state.show_virtual_drive_settings =
                crate::ui::components::virtual_drive_settings::render_virtual_drive_settings(
                    ctx,
                    self.navigation_state.show_virtual_drive_settings,
                );
        }

        // 12. Notifications
        app::notifications::render_notifications(self, ctx);

        // 13. Global Search Overlay (on top of everything)
        crate::ui::global_search_overlay::render_global_search_overlay(self, ctx);

        // Keep drag feedback on top and avoid cursor override by later widgets.
        if self.is_item_dragging {
            let (ctrl, shift, primary_released) = ctx.input(|i| {
                (
                    i.modifiers.ctrl,
                    i.modifiers.shift,
                    i.pointer.primary_released(),
                )
            });
            self.apply_item_drag_cursor_feedback(ctx);
            self.render_item_drag_preview(ctx, ctrl, shift);
            if primary_released {
                self.complete_item_drag(ctrl, shift);
            }
        }

        if is_in_size_move {
            // Ensure continuous redraw while the OS is in the modal move/resize loop.
            ctx.request_repaint();
        }

        // PERF: Log total frame time when slow (helps diagnose post-inactivity freezes)
        let frame_total_ms = t_frame_start.elapsed().as_millis();
        if frame_total_ms > 100 {
            log::warn!(
                "[PERF] SLOW FRAME: {}ms total (stable_dt={:.0}ms)",
                frame_total_ms, frame_ms
            );
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        app::lifecycle::handle_exit(self);
    }
}
