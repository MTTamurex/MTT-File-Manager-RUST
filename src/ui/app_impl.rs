use crate::app::ImageViewerApp;
use crate::infrastructure::windows::window_subclass::is_in_size_move;
use crate::ui::app;
use eframe::egui;

/// Periodic repaint interval to ensure drive bitmask checks run even when idle.
const DRIVE_BITMASK_REPAINT_MS: u64 = 1000;

impl eframe::App for ImageViewerApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let t_frame_start = std::time::Instant::now();

        // L-15: Only reset zoom when it has drifted (Ctrl+Scroll captured by input handler).
        // Avoids a no-op write to context state on every frame when zoom is already 1.0.
        if (ctx.zoom_factor() - 1.0).abs() > f32::EPSILON {
            ctx.set_zoom_factor(1.0);
        }

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

        // Use the larger of egui's stable_dt and the previous frame's actual render
        // time so that throttle guards react to real rendering cost, not just the
        // inter-frame interval reported by egui (which stays ~17ms even when a frame
        // actually took 200ms+ due to OS paging / GPU wake after inactivity).
        let effective_frame_ms = frame_ms.max(self.last_actual_frame_ms);

        if frame_ms > 0.0 {
            if self.frame_time_avg_ms <= 0.0 {
                self.frame_time_avg_ms = frame_ms;
            } else {
                self.frame_time_avg_ms = self.frame_time_avg_ms * 0.9 + frame_ms * 0.1;
            }
            if self.frame_time_peak_ms <= 0.0 {
                self.frame_time_peak_ms = effective_frame_ms;
            } else {
                // During restore burst, force the peak back to the average
                // immediately.  The slow first frames are caused by OS page
                // faults, not rendering load --- keeping peak inflated starves
                // upload budgets through the adaptive throttle guards and
                // prolongs the blank-tile period.
                let decay = if self.is_in_restore_burst() {
                    0.50
                } else if self.frame_time_peak_ms > 50.0 && self.frame_time_avg_ms < 25.0 {
                    // Transient wake spike (not burst): fast recovery
                    0.70
                } else {
                    0.95
                };
                self.frame_time_peak_ms *= decay;
                if effective_frame_ms > self.frame_time_peak_ms {
                    self.frame_time_peak_ms = effective_frame_ms;
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
            // During restore burst, repaint as fast as possible so textures re-populate
            // without waiting for user input or the 1-second idle timer.
            if self.is_in_restore_burst() {
                ctx.request_repaint();
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(DRIVE_BITMASK_REPAINT_MS));
            }
            let t2 = std::time::Instant::now();
            self.poll_drive_scan();
            self.poll_drive_info();
            // Poll sidebar folder tree background loads
            if self.sidebar_tree.poll_loaded() {
                ctx.request_repaint();
            }
            // Periodically re-enumerate expanded sidebar directories to catch
            // external changes (the per-folder notify watcher doesn't cover them).
            self.sidebar_tree.refresh_expanded_if_stale();

            let t3 = std::time::Instant::now();
            // Flush debounced preferences (max once per second)
            self.flush_preferences_if_needed();
            let t4 = std::time::Instant::now();
            // Bound long-session cache growth without disrupting interactive work.
            self.run_memory_maintenance();
            // Reap standalone video player process if it exited naturally.
            self.reap_video_player_process();
            let t5 = std::time::Instant::now();

            let msg_ms = t1.duration_since(t0).as_secs_f32() * 1000.0;
            let drives_ms = t2.duration_since(t1).as_secs_f32() * 1000.0;
            let poll_ms = t3.duration_since(t2).as_secs_f32() * 1000.0;
            let prefs_ms = t4.duration_since(t3).as_secs_f32() * 1000.0;
            let memory_ms = t5.duration_since(t4).as_secs_f32() * 1000.0;
            let infra_total = msg_ms + drives_ms + poll_ms + prefs_ms + memory_ms;
            if infra_total > 50.0 {
                log::warn!(
                    "[PERF] Slow infrastructure: messages={:.0}ms drives={:.0}ms poll={:.0}ms prefs={:.0}ms memory={:.0}ms",
                    msg_ms, drives_ms, poll_ms, prefs_ms, memory_ms
                );
            }
        }

        self.ensure_computer_icon(ctx);

        // Poll background icon extractions (sidebar drive/folder icons)
        self.item_icon_loader.poll_async_icons(ctx);

        // Poll shell menu worker results (async extraction / lazy submenu loading)
        if self.shell_menu_loading {
            use crate::infrastructure::shell_menu_worker::ShellMenuResponse;
            while let Ok(response) = self.shell_menu_res_rx.try_recv() {
                match response {
                    ShellMenuResponse::Ready { request_id, items } => {
                        if self.context_menu.is_open && request_id == self.shell_menu_request_id {
                            let ctx_clone = ctx.clone();
                            self.apply_async_shell_items(items, &ctx_clone);
                        }
                        if request_id == self.shell_menu_request_id {
                            self.shell_menu_loading = false;
                        }
                    }
                    ShellMenuResponse::Error {
                        request_id,
                        message,
                    } => {
                        if request_id == self.shell_menu_request_id {
                            log::debug!("[ShellMenu] Extraction error: {}", message);
                            self.notifications
                                .warning(rust_i18n::t!("context_menu.shell_menu_error").to_string());
                            self.shell_menu_loading = false;
                        }
                    }
                    ShellMenuResponse::SubmenuLoaded {
                        request_id,
                        item_id,
                        sub_items,
                    } => {
                        if request_id == self.shell_menu_request_id {
                            let ctx_clone = ctx.clone();
                            self.apply_async_submenu_items(item_id, sub_items, &ctx_clone);
                        }
                    }
                    ShellMenuResponse::Invoked => {}
                }
            }
        }

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
        let panels_ms = t_panels.elapsed().as_secs_f32() * 1000.0;
        if panels_ms > 50.0 {
            log::warn!("[PERF] Slow render_panels: {:.0}ms", panels_ms);
        }

        // 9. Operations: Context Menu (Rendering & Actions)
        app::menu_handler::handle_context_menu(self, ctx);

        // 10. Operations: Resize borders (on top) - REMOVED, handled by native subclass
        // app::input::handle_resize_borders(self, ctx);

        // 11. Settings window
        if self.navigation_state.show_settings_window {
            let output = crate::ui::components::settings_window::render_settings_window(
                ctx,
                self.navigation_state.show_settings_window,
                &mut self.navigation_state.active_settings_section,
                &mut self.theme_mode,
                &self.active_gpu_backend,
                &mut self.gpu_backend_preference,
                &mut self.shortcuts,
                &mut self.shortcut_editor,
            );
            self.navigation_state.show_settings_window = output.keep_open;
            if !output.keep_open {
                self.shortcut_editor.clear();
            }
            if output.theme_changed {
                match self.theme_mode {
                    crate::app::navigation_state::ThemeMode::Dark => ctx.set_visuals(egui::Visuals::dark()),
                    crate::app::navigation_state::ThemeMode::Light => ctx.set_visuals(egui::Visuals::light()),
                }
                crate::ui::theme::apply_scroll_style(ctx);
                self.save_preferences();
                self.force_save_preferences();
            }
            if output.language_changed {
                self.save_preferences();
                self.force_save_preferences();
            }
            if output.backend_changed {
                self.save_preferences();
                self.force_save_preferences();
            }
            if output.shortcuts_changed {
                self.save_preferences();
                self.force_save_preferences();
            }
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
        let frame_total_ms = t_frame_start.elapsed().as_secs_f32() * 1000.0;
        self.last_actual_frame_ms = frame_total_ms;

        if !self.layout.saved_is_minimized && frame_total_ms > 100.0 {
            log::warn!(
                "[PERF] SLOW FRAME: {:.0}ms total (stable_dt={:.0}ms)",
                frame_total_ms, frame_ms
            );
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        app::lifecycle::handle_exit(self);
    }

    fn persist_egui_memory(&self) -> bool {
        false
    }
}
