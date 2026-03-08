use crate::ui::components::MediaPreview;
use crate::ui::svg_icons::SvgIconManager;
use eframe::egui;

mod detached;
mod pickers;

/// Result from video control interactions.
pub enum ControlAction {
    /// User changed the volume slider.
    VolumeChanged(f32),
    /// User clicked the detach button while in docked mode.
    DetachRequested,
}

/// Helper: Create frameless button with hover effect
pub(super) fn add_icon_button(
    ui: &mut egui::Ui,
    tex: &egui::TextureHandle,
    btn_size: f32,
    tooltip: &str,
    dark_mode: bool,
) -> bool {
    let desired_size = egui::vec2(btn_size + 8.0, btn_size + 8.0);
    let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::click());

    // Draw hover background (rounded rect)
    if response.hovered() {
        let hover_color = if dark_mode {
            egui::Color32::from_white_alpha(25)
        } else {
            egui::Color32::from_black_alpha(15)
        };
        ui.painter().rect_filled(rect, 4.0, hover_color);
    }

    // Draw icon centered
    let icon_rect = egui::Rect::from_center_size(rect.center(), egui::vec2(btn_size, btn_size));
    ui.painter().image(
        tex.id(),
        icon_rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );

    response.on_hover_text(tooltip).clicked()
}

/// Get icon color based on dark mode
pub(super) fn icon_color(dark_mode: bool) -> [u8; 4] {
    if dark_mode {
        [240, 240, 240, 255]
    } else {
        [60, 60, 60, 255]
    }
}

/// Draw the seek bar
fn draw_seek_bar(
    ui: &mut egui::Ui,
    preview: &mut MediaPreview,
    full_width: f32,
    current_time: f64,
    duration: f64,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().slider_width = full_width;
        ui.visuals_mut().selection.bg_fill = crate::ui::theme::COLOR_ACCENT;

        let mut seek_value = current_time;
        if ui
            .add(
                egui::Slider::new(&mut seek_value, 0.0..=duration.max(0.1))
                    .show_value(false)
                    .trailing_fill(true),
            )
            .changed()
        {
            preview.seek(seek_value);
        }
    });
}

/// Draw basic controls: Play/Pause, Detach, Volume
/// Returns a `ControlAction` if the user interacted with a control.
fn draw_basic_controls(
    ui: &mut egui::Ui,
    preview: &mut MediaPreview,
    svg_manager: &mut SvgIconManager,
    is_playing: bool,
    volume: f32,
    is_muted: bool,
    is_detached: bool,
) -> Option<ControlAction> {
    let icon_color_val = icon_color(ui.visuals().dark_mode);
    let btn_size = 18.0;

    // Play/Pause
    let play_icon = if is_playing { "pause" } else { "play" };
    if let Some(tex) = svg_manager.get_icon(ui.ctx(), play_icon, 48, icon_color_val) {
        let tooltip = if is_playing { rust_i18n::t!("video.pause") } else { rust_i18n::t!("video.play") };
        if add_icon_button(ui, &tex, btn_size, &tooltip, ui.visuals().dark_mode) {
            preview.toggle_play();
        }
    }

    ui.add_space(2.0);

    // Detach Button
    let detach_icon_name = if is_detached {
        "minimize_2"
    } else {
        "external-link"
    };
    let detach_tooltip = if is_detached {
        rust_i18n::t!("video.attach")
    } else {
        rust_i18n::t!("video.detach")
    };
    if let Some(tex) = svg_manager.get_icon(ui.ctx(), detach_icon_name, 48, icon_color_val) {
        if add_icon_button(ui, &tex, btn_size, &detach_tooltip, ui.visuals().dark_mode) {
            if !is_detached {
                // In docked mode: signal a detach request (spawns standalone process)
                return Some(ControlAction::DetachRequested);
            } else {
                // In detached mode (standalone controls): re-dock
                if preview.is_maximized() {
                    preview.set_fullscreen_applied(false);
                    preview.set_forced_size(None);
                    preview.reset_last_rect();
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
                    if preview.prev_app_maximized() {
                        ui.ctx()
                            .send_viewport_cmd(egui::ViewportCommand::Maximized(true));
                    }
                }
                preview.toggle_detached();
            }
        }
    }

    ui.add_space(2.0);

    // Volume Button (Mute/Unmute)
    // CRASH FIX: Use try-lock pattern to avoid deadlock with MPV thread
    let vol_icon = if is_muted { "vol_mute" } else { "vol_high" };
    if let Some(tex) = svg_manager.get_icon(ui.ctx(), vol_icon, 48, icon_color_val) {
        let vol_tooltip = if is_muted { rust_i18n::t!("video.unmute") } else { rust_i18n::t!("video.mute_btn") };
        if add_icon_button(ui, &tex, btn_size, &vol_tooltip, ui.visuals().dark_mode) {
            // SAFETY: Wrap in catch_unwind to prevent crash from FFI/panic
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                preview.toggle_mute();
            }));
        }
    }

    // Volume Slider
    let mut vol = volume;
    ui.add_space(4.0);
    ui.spacing_mut().slider_width = 70.0;
    ui.visuals_mut().selection.bg_fill = crate::ui::theme::COLOR_ACCENT;
    if ui
        .add(egui::Slider::new(&mut vol, 0.0..=1.0).show_value(false))
        .changed()
    {
        // SAFETY: Wrap in catch_unwind to prevent crash from FFI/panic
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            preview.set_volume(vol);
        }));
        return Some(ControlAction::VolumeChanged(vol));
    }
    None
}

/// Draw time display
fn draw_time_display(ui: &mut egui::Ui, current_time: f64, duration: f64) {
    let time_text = format!(
        "{} / {}",
        crate::ui::components::media_preview::format_time(current_time),
        crate::ui::components::media_preview::format_time(duration)
    );
    let time_color = if ui.visuals().dark_mode {
        egui::Color32::LIGHT_GRAY
    } else {
        egui::Color32::DARK_GRAY
    };
    ui.label(egui::RichText::new(time_text).size(12.0).color(time_color));
}

/// Draw simple controls for docked mode (preview panel)
/// Shows only: seek bar, play/pause, detach, volume, time
/// Returns a `ControlAction` if the user interacted with a control.
#[allow(clippy::too_many_arguments)]
pub fn draw_docked_controls(
    ui: &mut egui::Ui,
    preview: &mut MediaPreview,
    full_width: f32,
    svg_manager: &mut SvgIconManager,
    is_playing: bool,
    current_time: f64,
    duration: f64,
    volume: f32,
    is_muted: bool,
) -> Option<ControlAction> {
    ui.set_width(full_width);

    // Seek Bar
    draw_seek_bar(ui, preview, full_width, current_time, duration);

    ui.add_space(6.0);

    // Buttons row - only basic controls for docked mode
    let mut result = None;
    ui.horizontal(|ui| {
        // Basic controls (play, detach, volume)
        result = draw_basic_controls(
            ui,
            preview,
            svg_manager,
            is_playing,
            volume,
            is_muted,
            false,
        );

        ui.add_space(10.0);

        // Time display
        draw_time_display(ui, current_time, duration);
    });
    result
}

/// Draw full controls for detached mode (floating window)
/// Shows all controls including audio/subtitle pickers, normalizer, fullscreen, VSR
/// Returns a `ControlAction` if the user interacted with a control.
#[allow(clippy::too_many_arguments)]
pub fn draw_detached_controls(
    ui: &mut egui::Ui,
    preview: &mut MediaPreview,
    full_width: f32,
    svg_manager: &mut SvgIconManager,
    is_playing: bool,
    current_time: f64,
    duration: f64,
    volume: f32,
    is_muted: bool,
) -> Option<ControlAction> {
    ui.set_width(full_width);

    // Seek Bar
    draw_seek_bar(ui, preview, full_width, current_time, duration);

    ui.add_space(6.0);

    // Buttons row - full controls for detached mode
    let mut result = None;
    ui.horizontal(|ui| {
        // Basic controls (play, detach, volume)
        result = draw_basic_controls(ui, preview, svg_manager, is_playing, volume, is_muted, true);

        ui.add_space(10.0);

        // Time display
        draw_time_display(ui, current_time, duration);

        ui.add_space(8.0);

        // Audio track picker (only in detached mode)
        pickers::draw_audio_track_picker(ui, preview);

        // Subtitle track picker (only in detached mode)
        pickers::draw_subtitle_track_picker(ui, preview, svg_manager);

        // Audio normalizer (only in detached mode)
        detached::draw_audio_normalizer(ui, preview, svg_manager);

        // Right-aligned buttons (fullscreen, VSR)
        detached::draw_detached_buttons(ui, preview, svg_manager);
    });
    result
}

/// Legacy function for backward compatibility - delegates to appropriate version
/// Returns a `ControlAction` if the user interacted with a control.
#[allow(clippy::too_many_arguments)]
pub fn draw_video_controls(
    ui: &mut egui::Ui,
    preview: &mut MediaPreview,
    full_width: f32,
    svg_manager: &mut SvgIconManager,
    is_playing: bool,
    current_time: f64,
    duration: f64,
    volume: f32,
    is_muted: bool,
    is_detached: bool,
) -> Option<ControlAction> {
    if is_detached {
        draw_detached_controls(
            ui,
            preview,
            full_width,
            svg_manager,
            is_playing,
            current_time,
            duration,
            volume,
            is_muted,
        )
    } else {
        draw_docked_controls(
            ui,
            preview,
            full_width,
            svg_manager,
            is_playing,
            current_time,
            duration,
            volume,
            is_muted,
        )
    }
}
