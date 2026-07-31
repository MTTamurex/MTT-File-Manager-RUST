use crate::app::ImageViewerApp;
use crate::infrastructure::windows::window_subclass::is_in_size_move;
use crate::ui::app;
use eframe::egui;
use std::path::{Path, PathBuf};
use std::time::Duration;

const FILE_OP_REPAINT_INTERVAL: Duration = Duration::from_millis(100);
const PREFERENCES_FLUSH_INTERVAL: Duration = Duration::from_secs(1);
const CLIPBOARD_CLEANUP_RETRY_INTERVAL: Duration = Duration::from_secs(1);
const DRIVE_BITMASK_CHECK_INTERVAL: Duration = Duration::from_secs(3);
const DRIVE_INFO_REFRESH_INTERVAL: Duration = Duration::from_secs(5);

struct RepaintDeadlineInputs {
    file_ops_in_progress: bool,
    clipboard_cleanup_pending: bool,
    preferences_save_elapsed: Option<Duration>,
    drive_refresh_elapsed: Duration,
    drive_info_refresh_elapsed: Option<Duration>,
}

fn remaining_after_frame_check(elapsed: Duration, interval: Duration) -> Duration {
    if elapsed >= interval {
        interval
    } else {
        interval - elapsed
    }
}

fn next_background_repaint(inputs: RepaintDeadlineInputs) -> Duration {
    let mut deadline =
        remaining_after_frame_check(inputs.drive_refresh_elapsed, DRIVE_BITMASK_CHECK_INTERVAL);

    if inputs.file_ops_in_progress {
        deadline = deadline.min(FILE_OP_REPAINT_INTERVAL);
    }
    if inputs.clipboard_cleanup_pending {
        deadline = deadline.min(CLIPBOARD_CLEANUP_RETRY_INTERVAL);
    }
    if let Some(elapsed) = inputs.preferences_save_elapsed {
        deadline = deadline.min(remaining_after_frame_check(
            elapsed,
            PREFERENCES_FLUSH_INTERVAL,
        ));
    }
    if let Some(elapsed) = inputs.drive_info_refresh_elapsed {
        deadline = deadline.min(remaining_after_frame_check(
            elapsed,
            DRIVE_INFO_REFRESH_INTERVAL,
        ));
    }

    deadline
}

impl ImageViewerApp {
    fn request_next_background_repaint(&self, ctx: &egui::Context) {
        let local_scope = crate::app::drive_state::DriveInfoRefreshScope::Local;
        let drive_info_refresh_elapsed = (self.show_left_sidebar
            || self.navigation_state.is_computer_view)
            .then(|| {
                (!self.drive_state.drive_info_refresh.is_pending(local_scope))
                    .then(|| self.drive_state.drive_info_refresh.elapsed(local_scope))
            })
            .flatten();
        let deadline = next_background_repaint(RepaintDeadlineInputs {
            file_ops_in_progress: self.file_operation_state.file_ops_in_progress > 0,
            clipboard_cleanup_pending: self
                .file_operation_state
                .pending_clipboard_cleanup_sequence
                .is_some(),
            preferences_save_elapsed: self
                .preferences_dirty
                .then(|| self.preferences_last_save.elapsed()),
            drive_refresh_elapsed: self.drive_state.last_drive_refresh.elapsed(),
            drive_info_refresh_elapsed,
        });
        ctx.request_repaint_after(deadline);
    }

    fn run_background_updates(&mut self, ctx: &egui::Context, upload_textures: bool) {
        if is_in_size_move() {
            self.refresh_working_set_trim_blocker(true);
            return;
        }

        let t0 = std::time::Instant::now();
        if upload_textures {
            self.process_incoming_messages(ctx);
        } else {
            self.process_background_messages(ctx);
            self.retry_completed_clipboard_cleanup();
            self.flush_preferences_if_needed();
            self.refresh_working_set_trim_blocker(false);
            self.reap_video_player_process();
            crate::viewer_processes::reap_exited();
            self.request_next_background_repaint(ctx);
            return;
        }
        self.retry_completed_clipboard_cleanup();
        let t1 = std::time::Instant::now();
        if self.file_operation_state.file_ops_in_progress == 0 {
            self.refresh_drives_if_needed();
        }

        let t2 = std::time::Instant::now();
        self.poll_drive_scan();
        self.poll_drive_info();
        if self.sidebar_tree.poll_loaded() {
            ctx.request_repaint();
        }
        if self.miller_columns.poll() {
            ctx.request_repaint();
        }
        if self.file_operation_state.file_ops_in_progress == 0 {
            self.sidebar_tree.refresh_expanded_if_stale();
        }

        let t3 = std::time::Instant::now();
        self.flush_preferences_if_needed();
        let t4 = std::time::Instant::now();
        self.run_memory_maintenance();
        self.maybe_log_memory_snapshot("frame");
        self.reap_video_player_process();
        crate::viewer_processes::reap_exited();
        let t5 = std::time::Instant::now();

        // Keep logic polling alive while idle or while eframe suppresses UI for
        // an occluded viewport, without forcing a fixed repaint cadence.
        if self.is_in_restore_burst() {
            if self.is_opengl_backend() {
                ctx.request_repaint_after(Duration::from_millis(16));
            } else {
                ctx.request_repaint();
            }
        } else {
            self.request_next_background_repaint(ctx);
        }

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
}

impl eframe::App for ImageViewerApp {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // Startup must advance while the root viewport is still hidden.
        app::lifecycle::handle_startup_sequence(self, ctx);
        app::lifecycle::track_window_state(self, ctx);
        self.ensure_window_handle(frame);
        let viewport_visible = ctx.input(|input| input.viewport().visible().unwrap_or(true));
        if !viewport_visible {
            self.run_background_updates(ctx, false);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx_owned = ui.ctx().clone();
        let ctx = &ctx_owned;
        ui.set_style(ctx.global_style());
        let t_frame_start = std::time::Instant::now();

        // L-15: Only reset zoom when it has drifted (Ctrl+Scroll captured by input handler).
        // Avoids a no-op write to context state on every frame when zoom is already 1.0.
        if (ctx.zoom_factor() - 1.0).abs() > f32::EPSILON {
            ctx.set_zoom_factor(1.0);
        }

        let primary_press_received_by_egui = ctx.input(|i| {
            i.events.iter().any(|event| {
                matches!(
                    event,
                    egui::Event::PointerButton {
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        ..
                    }
                )
            })
        });
        self.update_outbound_drag_input_guard(primary_press_received_by_egui);

        // True while Windows is in interactive move/resize loop (WM_ENTERSIZEMOVE..EXITSIZEMOVE).
        let is_in_size_move = is_in_size_move();

        self.run_background_updates(ctx, true);

        // Apply asynchronously loaded fonts as soon as the worker finishes.
        if let Some(rx) = &self.font_loader_rx {
            if let Ok(fonts) = rx.try_recv() {
                ctx.set_fonts(fonts);
                self.font_loader_rx = None; // Disable loader once done
                ctx.request_repaint(); // Force refresh with new fonts
            }
        }

        // 1. Initial validation
        if self.startup_tick == 0 {

            // NOTE: Removed path.exists() check here because it can BLOCK indefinitely
            // on OneDrive cloud-only files, causing UI freeze. The file selection will
            // be cleared naturally when the user navigates away or refreshes the folder.
            // If we need this check, it should be done asynchronously in a worker thread.
        }

        self.auto_disable_diagnostic_mode_if_needed();
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

        self.ensure_computer_icon(ctx);

        // Poll background icon extractions (sidebar drive/folder icons)
        self.item_icon_loader.poll_async_icons(ctx);

        // Poll shell menu worker results (async extraction / lazy submenu loading)
        {
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
                            self.notifications.warning(
                                rust_i18n::t!("context_menu.shell_menu_error").to_string(),
                            );
                            // Remove the loading placeholder on error so the menu doesn't
                            // keep showing a stale "Loading…" item.
                            self.context_menu
                                .items
                                .retain(|item| !item.is_loading_placeholder);
                            // Remove any trailing separator that preceded the placeholder.
                            while self
                                .context_menu
                                .items
                                .last()
                                .is_some_and(|item| item.is_separator)
                            {
                                self.context_menu.items.pop();
                            }
                            self.context_menu.partition_items();
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
                            self.shell_menu_loading = false;
                        }
                    }
                    ShellMenuResponse::Invoked { request_id } => {
                        if request_id == self.shell_menu_request_id {
                            self.shell_menu_loading = false;
                        }
                        if self.global_search.shell_refresh_request_id == Some(request_id) {
                            self.global_search.shell_refresh_request_id = None;
                            self.request_global_search_refresh();
                        }
                    }
                }
            }
        }

        // 4. External drag-and-drop: detect hover from other applications (WinRAR, Explorer)
        //    Must run BEFORE panels render so item renderers can show folder highlights.
        if !self.is_item_dragging {
            let (has_external_hover, has_external_drop) = ctx.input(|i| {
                (
                    !i.raw.hovered_files.is_empty(),
                    !i.raw.dropped_files.is_empty(),
                )
            });
            if has_external_hover || has_external_drop {
                self.external_drop_active = true;
                self.drag_target_folder = None;
                self.drag_hovered_folder = None;
                self.external_drop_inactive_folder = None;
            } else if self.external_drop_active {
                // Hover ended without a drop (drag left window) or drop was already consumed.
                self.reset_external_drop_state();
            }
        }

        // 5. Input: Keyboard shortcuts (resize borders handled by native subclass)
        if !is_in_size_move {
            app::input::handle_input(self, ctx);
        }

        // 6. Layout: Status Bar (Bottom) - lightweight, always render
        app::layers::render_status_bar_layer(self, ui);

        // 7. Layout: Tab Bar (Top 1) - lightweight, always render
        app::layers::render_tab_bar_layer(self, ui, frame);

        // 8. Layout: Toolbar (Top 2) - lightweight, always render
        app::layers::render_toolbar_layer(self, ui);

        // 8b. Layout: Secondary Toolbar (Top 3) - lightweight, always render
        app::layers::render_secondary_toolbar_layer(self, ui);

        // 8c. Settings backdrop (rendered BEFORE panels to block their input)
        let settings_close_from_backdrop = if self.navigation_state.show_settings_window {
            crate::ui::components::settings_window::render_settings_backdrop(ctx)
        } else {
            false
        };

        // 9. Layout: Main Panels (Sidebar, Preview, Central)
        // Keep full rendering even during move/resize so content/video stays visible and synchronized.
        let t_panels = std::time::Instant::now();
        app::panels::render_panels(self, ui, frame);
        let panels_ms = t_panels.elapsed().as_secs_f32() * 1000.0;
        if panels_ms > 50.0 {
            log::warn!("[PERF] Slow render_panels: {:.0}ms", panels_ms);
        }

        // 10. Operations: Context Menu (Rendering & Actions)
        if !self.global_search.active {
            app::menu_handler::handle_context_menu(self, ctx);
        }

        // 11. Operations: Resize borders (on top) - REMOVED, handled by native subclass
        // app::input::handle_resize_borders(self, ctx);

        // 12. Settings window
        if self.navigation_state.show_settings_window {
            let output = crate::ui::components::settings_window::render_settings_window(
                ctx,
                self,
                settings_close_from_backdrop,
            );
            self.navigation_state.show_settings_window = output.keep_open;
            if !output.keep_open {
                self.shortcut_editor.clear();
            }
            if output.theme_changed {
                match self.theme_mode {
                    crate::app::navigation_state::ThemeMode::Dark => {
                        ctx.set_visuals(egui::Visuals::dark())
                    }
                    crate::app::navigation_state::ThemeMode::Light => {
                        ctx.set_visuals(egui::Visuals::light())
                    }
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
            if output.quick_access_changed {
                self.save_preferences();
                self.force_save_preferences();
            }
            if output.recycle_bin_changed {
                self.save_preferences();
                self.force_save_preferences();
            }
            if output.tags_changed {
                self.save_preferences();
                self.force_save_preferences();
            }
            if output.diagnostic_mode_changed {
                self.set_diagnostic_mode(self.diagnostic_mode);
            }
            if output.open_diagnostic_folder {
                self.open_diagnostic_log_folder();
            }
        }

        // 13. Batch Rename Modal
        if self.batch_rename_state.is_some() {
            crate::ui::components::batch_rename_modal::render_batch_rename_modal(self, ctx);
        }

        if self.show_tag_manager {
            crate::ui::components::tag_manager_modal::render_tag_manager_modal(self, ctx);
        }

        // 14. Notifications
        app::notifications::render_notifications(self, ctx);

        // 15. Global Search Overlay (on top of everything)
        crate::ui::global_search_overlay::render_global_search_overlay(self, ctx);

        if self.global_search.active {
            app::menu_handler::handle_context_menu(self, ctx);
        }

        if self.pending_drag_move_confirmation.is_some() && self.is_item_dragging {
            self.cancel_item_drag();
        }

        // Keep drag feedback on top and avoid cursor override by later widgets.
        if self.is_item_dragging
            && !self.file_panel_input_blocked_by_drag_move_confirmation()
            && !self.try_start_outbound_item_drag()
        {
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

        if self.pending_drag_move_confirmation.is_some() {
            crate::ui::components::drag_move_confirmation_modal::render_drag_move_confirmation_modal(
                self, ctx,
            );
        }

        // Consume external file drops (from other applications such as WinRAR/Explorer).
        // DroppedFile events arrive on the same frame that hovered_files is cleared.
        // The bridge renderers already resolved drag_target_folder via
        // update_external_drop_hover during panel rendering.
        let dropped_files: Vec<egui::DroppedFile> =
            ctx.input_mut(|i| std::mem::take(&mut i.raw.dropped_files));
        if !dropped_files.is_empty() {
            let source_paths: Vec<PathBuf> = dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect();

            if !source_paths.is_empty() {
                // Prefer the folder under the cursor (from bridge hover tracking),
                // fall back to the inactive panel folder in dual-panel mode,
                // and finally to the current directory.
                if let Some(dest_folder) = resolve_external_drop_destination(self) {
                    log::info!(
                        "[ExternalDrop] Dropping {} file(s) into '{}'",
                        source_paths.len(),
                        dest_folder.display()
                    );
                    let hwnd = self.shell_op_hwnd();
                    let request =
                        crate::workers::file_operation_worker::FileOperationRequest::copy_batch(
                            source_paths,
                            dest_folder,
                            hwnd,
                        );
                    self.file_operation_state.file_ops_in_progress += 1;
                    if self
                        .file_operation_state
                        .file_op_sender
                        .send(request)
                        .is_err()
                    {
                        self.file_operation_state.file_ops_in_progress = self
                            .file_operation_state
                            .file_ops_in_progress
                            .saturating_sub(1);
                        log::warn!("[ExternalDrop] file operation worker channel closed");
                    }
                } else {
                    log::warn!(
                        "[ExternalDrop] No valid drop destination; ignoring {} files",
                        source_paths.len()
                    );
                }
            }
            self.reset_external_drop_state();
        }

        // PERF: Log total frame time when slow (helps diagnose post-inactivity freezes)
        let frame_total_ms = t_frame_start.elapsed().as_secs_f32() * 1000.0;
        self.last_actual_frame_ms = frame_total_ms;

        if !self.layout.saved_is_minimized && frame_total_ms > 100.0 {
            log::warn!(
                "[PERF] SLOW FRAME: {:.0}ms total (stable_dt={:.0}ms)",
                frame_total_ms,
                frame_ms
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

fn resolve_external_drop_destination(app: &ImageViewerApp) -> Option<PathBuf> {
    app.drag_target_folder
        .clone()
        .filter(|target| is_valid_external_drop_destination(target))
        .or_else(|| {
            app.external_drop_inactive_folder
                .clone()
                .filter(|target| is_valid_external_drop_destination(target))
        })
        .or_else(|| {
            if app.navigation_state.is_recycle_bin_view || app.navigation_state.is_computer_view {
                return None;
            }

            let current = PathBuf::from(&app.navigation_state.current_path);
            is_valid_external_drop_destination(&current).then_some(current)
        })
}

fn is_valid_external_drop_destination(target: &Path) -> bool {
    !target.as_os_str().is_empty()
        && !target
            .to_str()
            .is_some_and(crate::domain::special_paths::is_virtual_path)
        && !ImageViewerApp::path_is_archive_namespace(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idle_inputs() -> RepaintDeadlineInputs {
        RepaintDeadlineInputs {
            file_ops_in_progress: false,
            clipboard_cleanup_pending: false,
            preferences_save_elapsed: None,
            drive_refresh_elapsed: Duration::ZERO,
            drive_info_refresh_elapsed: None,
        }
    }

    #[test]
    fn repaint_uses_earliest_pending_deadline() {
        let inputs = RepaintDeadlineInputs {
            preferences_save_elapsed: None,
            drive_refresh_elapsed: Duration::from_secs(1),
            drive_info_refresh_elapsed: Some(Duration::from_millis(4500)),
            ..idle_inputs()
        };

        assert_eq!(next_background_repaint(inputs), Duration::from_millis(500));
    }

    #[test]
    fn dirty_preferences_keep_the_one_second_debounce_deadline() {
        let inputs = RepaintDeadlineInputs {
            preferences_save_elapsed: Some(Duration::from_millis(250)),
            ..idle_inputs()
        };

        assert_eq!(next_background_repaint(inputs), Duration::from_millis(750));
    }

    #[test]
    fn file_operations_keep_the_100ms_poll_deadline() {
        let inputs = RepaintDeadlineInputs {
            file_ops_in_progress: true,
            ..idle_inputs()
        };

        assert_eq!(next_background_repaint(inputs), Duration::from_millis(100));
    }

    #[test]
    fn pending_clipboard_cleanup_keeps_retry_alive() {
        let inputs = RepaintDeadlineInputs {
            clipboard_cleanup_pending: true,
            ..idle_inputs()
        };

        assert_eq!(next_background_repaint(inputs), Duration::from_secs(1));
    }

    #[test]
    fn idle_repaint_tracks_drive_bitmask_deadline() {
        let inputs = RepaintDeadlineInputs {
            drive_refresh_elapsed: Duration::from_millis(750),
            ..idle_inputs()
        };

        assert_eq!(next_background_repaint(inputs), Duration::from_millis(2250));
    }

    #[test]
    fn completed_checks_start_a_new_interval_instead_of_a_hot_loop() {
        assert_eq!(
            remaining_after_frame_check(Duration::from_secs(30), Duration::from_secs(3)),
            Duration::from_secs(3)
        );
    }
}
