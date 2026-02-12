use crate::ui::components::MediaPreview;
use crate::ui::preview_panel::video_preview::controls::draw_video_controls;
use crate::ui::svg_icons::SvgIconManager;
use eframe::egui;

#[allow(clippy::too_many_arguments)]
pub fn render_detached_video(
    ui: &mut egui::Ui,
    preview: &mut MediaPreview,
    svg_manager: &mut SvgIconManager,
    frame: Option<&eframe::Frame>,
    filename: &str,
    current_time: f64,
    duration: f64,
    volume: f32,
    is_muted: bool,
    is_playing: bool,
) {
    // 2. Floating Window logic
    let mut open = true;
    let is_fullscreen = preview.is_maximized(); // Renamed for clarity: this is now fullscreen
    let mut should_restore = preview.should_restore();
    let last_known_rect = preview.get_last_window_rect();

    // Detect minimize/restore cycle to preserve window position
    let is_minimized = ui.ctx().input(|i| i.viewport().minimized.unwrap_or(false));
    if is_minimized && !preview.was_minimized() {
        // Just got minimized - mark it
        preview.set_was_minimized(true);
    } else if !is_minimized && preview.was_minimized() {
        // Just got restored from minimize - force window restore
        preview.set_was_minimized(false);
        if last_known_rect.is_some() {
            should_restore = true;
            preview.set_restore_needed(true);
        }
    }

    if is_fullscreen {
        // Fallback or coordinate with fullscreen.rs?
        // Actually, render_preview_panel calls this. If it's fullscreen, it should probably call fullscreen.rs logic.
        // I'll leave a note or handle it here if it's easier.
        // For now, let's keep the windowed logic here.
    }

    // Restore from fullscreen if needed
    if preview.fullscreen_applied() && !is_fullscreen {
        preview.set_fullscreen_applied(false);
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
        if preview.prev_app_maximized() {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Maximized(true));
        }
    }

    // Condition Window Builder
    // Minimum width to fit all controls properly (increased to prevent overlap)
    let min_window_width = 780.0;
    let min_window_height = 450.0;
    let default_window_width = 800.0;
    let default_window_height = 500.0;

    let mut window_builder = egui::Window::new(filename)
        .open(&mut open)
        .collapsible(false)
        .title_bar(true)
        // Fix Z-Order overlap with Resize Handles (which are Foreground)
        .order(egui::Order::Foreground)
        .min_size([min_window_width, min_window_height]);

    if should_restore {
        // Force restoration to previous size for one frame
        if let Some(rect) = last_known_rect {
            // Ensure restored size respects minimum
            let w = rect.width().max(min_window_width);
            let h = rect.height().max(min_window_height);
            window_builder = window_builder.current_pos(rect.min).fixed_size([w, h]);
        } else {
            let screen = ui.ctx().screen_rect();
            let center = screen.center();
            let w = default_window_width;
            let h = default_window_height;
            let rect = egui::Rect::from_min_size(
                egui::pos2(center.x - w / 2.0, center.y - h / 2.0),
                egui::vec2(w, h),
            );
            window_builder = window_builder.current_pos(rect.min).fixed_size(rect.size());
        }
    } else {
        // Normal Floating State - use last known position if available
        if let Some(rect) = last_known_rect {
            // Ensure restored size respects minimum
            let w = rect.width().max(min_window_width);
            let h = rect.height().max(min_window_height);
            window_builder = window_builder
                .default_pos(rect.min)
                .default_size([w, h])
                .resizable(true);
        } else {
            window_builder = window_builder
                .default_size([default_window_width, default_window_height])
                .resizable(true);
        }
    }

    let use_native_osc = preview.native_osc_active();

    let window_response = window_builder.show(ui.ctx(), |ui| {
        // === TRUE AUTOHIDE IMPLEMENTATION ===
        // Video takes 100% when idle, shrinks when controls are shown

        let total_rect = ui.available_rect_before_wrap();
        let total_size = total_rect.size();

        // Control area rect (always calculate for hover detection)
        let control_height_base = 75.0;
        let control_rect_hover = egui::Rect::from_min_size(
            egui::pos2(total_rect.min.x, total_rect.max.y - control_height_base),
            egui::vec2(total_size.x, control_height_base),
        );

        if !use_native_osc {
            // Check if mouse is over control area and keep visible
            let mouse_over_controls = ui.input(|i| {
                i.pointer
                    .hover_pos()
                    .map(|p| control_rect_hover.contains(p))
                    .unwrap_or(false)
            });

            if mouse_over_controls {
                preview.reset_mouse_activity();
            }
        }

        // Determine if controls should be visible
        // Primary: MPV area reports mouse activity
        let show_controls = if use_native_osc {
            false
        } else {
            preview.controls_active()
        };

        // Control bar height (only when visible)
        let control_height = if show_controls { 75.0 } else { 0.0 };

        // Video takes remaining space
        let video_height = total_size.y - control_height;

        let video_rect =
            egui::Rect::from_min_size(total_rect.min, egui::vec2(total_size.x, video_height));

        // Allocate the total space with click detection
        let video_response = ui.allocate_exact_size(total_size, egui::Sense::click());

        // Double-click on video area to enter fullscreen
        // Only set flags here — the actual ViewportCommand::Fullscreen(true)
        // is sent from render_fullscreen_video() on the next frame
        if video_response.1.double_clicked() {
            let was_maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
            preview.set_prev_app_maximized(was_maximized);
            preview.set_fullscreen_applied(false);
            preview.toggle_maximized();
        }

        // Keyboard shortcuts: volume (Up/Down) and seek (Left/Right)
        // OSD feedback rendered natively by MPV via show-text command
        let vol_step = 0.05_f32;
        let seek_step = 5.0_f64;
        let osd_duration_ms = 2000_i64;

        if !use_native_osc {
            if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
                let new_vol = (volume + vol_step).min(1.0);
                preview.set_volume(new_vol);
                let msg = format!("Volume: {}%", (new_vol * 100.0).round() as i32);
                preview.show_osd(&msg, osd_duration_ms);
                preview.reset_mouse_activity();
            }
            if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
                let new_vol = (volume - vol_step).max(0.0);
                preview.set_volume(new_vol);
                let msg = format!("Volume: {}%", (new_vol * 100.0).round() as i32);
                preview.show_osd(&msg, osd_duration_ms);
                preview.reset_mouse_activity();
            }
            if ui.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
                let new_time = (current_time + seek_step).min(duration);
                preview.seek(new_time);
                let display_time = crate::ui::components::media_preview::format_time(new_time);
                let display_dur = crate::ui::components::media_preview::format_time(duration);
                let msg = format!("{} / {}", display_time, display_dur);
                preview.show_osd(&msg, osd_duration_ms);
                preview.reset_mouse_activity();
            }
            if ui.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
                let new_time = (current_time - seek_step).max(0.0);
                preview.seek(new_time);
                let display_time = crate::ui::components::media_preview::format_time(new_time);
                let display_dur = crate::ui::components::media_preview::format_time(duration);
                let msg = format!("{} / {}", display_time, display_dur);
                preview.show_osd(&msg, osd_duration_ms);
                preview.reset_mouse_activity();
            }
        }

        // 1. Render Video (full height when controls hidden)
        let mut video_ui = ui.new_child(egui::UiBuilder::new().max_rect(video_rect));
        preview.set_forced_size(Some(video_rect.size()));
        preview.show(&mut video_ui, frame);

        // 2. Render Controls only when active
        if show_controls {
            let control_rect = egui::Rect::from_min_size(
                egui::pos2(total_rect.min.x, total_rect.min.y + video_height),
                egui::vec2(total_size.x, control_height),
            );

            // Background - use theme-aware colors
            let bg_color = if ui.visuals().dark_mode {
                egui::Color32::from_rgb(35, 35, 38) // Dark mode panel background
            } else {
                egui::Color32::from_rgb(245, 245, 248) // Light mode panel background
            };
            ui.painter().rect_filled(control_rect, 0.0, bg_color);

            let mut control_ui = ui.new_child(egui::UiBuilder::new().max_rect(control_rect));
            control_ui.add_space(6.0);
            draw_video_controls(
                &mut control_ui,
                preview,
                control_rect.width() - 20.0,
                svg_manager,
                is_playing,
                current_time,
                duration,
                volume,
                is_muted,
                true,
            );
        }

        // Request repaint to check timeout and hide controls
        // Only repaint when video is playing or controls visible (perf optimization)
        if is_playing || preview.controls_active() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(200));
        }
    });

    // Post-Show Logic (only for windowed mode)
    // 1. If Normal State, update last_known_rect
    // PROTECT: Only save if NOT minimized AND size is valid (>50px) to prevent "squashed" restore
    if !should_restore && !is_minimized {
        if let Some(inner) = &window_response {
            let r = inner.response.rect;
            if r.width() > 50.0 && r.height() > 50.0 {
                preview.set_last_window_rect(r);
            }
        }
    }

    // 2. Clear Restore Flag
    if should_restore {
        preview.complete_restore();
    }

    // Handle close
    if !open {
        if preview.is_maximized() {
            preview.set_fullscreen_applied(false);
            preview.set_forced_size(None); // Clear forced size when exiting fullscreen
            preview.reset_last_rect(); // Force MPV window resize
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            if preview.prev_app_maximized() {
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::Maximized(true));
            }
        }
        preview.set_detached(false);
    }

    // ESC in detached window mode should re-dock only when player/app is focused.
    if open && !preview.is_maximized() && ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) {
        let app_focused = ui.ctx().input(|i| i.viewport().focused.unwrap_or(false));

        #[cfg(target_os = "windows")]
        let player_focused = {
            use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
            let foreground = unsafe { GetForegroundWindow() };
            !foreground.is_invalid() && preview.get_hwnd() == Some(foreground)
        };

        #[cfg(not(target_os = "windows"))]
        let player_focused = false;

        if app_focused || player_focused {
            preview.set_detached(false);
        }
    }
}
