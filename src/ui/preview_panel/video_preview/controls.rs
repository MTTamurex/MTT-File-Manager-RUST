use crate::ui::components::MediaPreview;
use crate::ui::preview_panel::utils::truncate_text_to_fit;
use crate::ui::svg_icons::SvgIconManager;
use eframe::egui;

/// Helper: Create frameless button with hover effect
pub fn add_icon_button(
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
    let icon_rect =
        egui::Rect::from_center_size(rect.center(), egui::vec2(btn_size, btn_size));
    ui.painter().image(
        tex.id(),
        icon_rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );

    response.on_hover_text(tooltip).clicked()
}

/// Get icon color based on dark mode
fn icon_color(dark_mode: bool) -> [u8; 4] {
    if dark_mode {
        [240, 240, 240, 255]
    } else {
        [60, 60, 60, 255]
    }
}

/// Draw the seek bar
fn draw_seek_bar(ui: &mut egui::Ui, preview: &mut MediaPreview, full_width: f32, current_time: f64, duration: f64) {
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
fn draw_basic_controls(
    ui: &mut egui::Ui,
    preview: &mut MediaPreview,
    svg_manager: &mut SvgIconManager,
    is_playing: bool,
    volume: f32,
    is_muted: bool,
    is_detached: bool,
) {
    let icon_color_val = icon_color(ui.visuals().dark_mode);
    let btn_size = 18.0;

    // Play/Pause
    let play_icon = if is_playing { "pause" } else { "play" };
    if let Some(tex) = svg_manager.get_icon(ui.ctx(), play_icon, 48, icon_color_val) {
        let tooltip = if is_playing { "Pausar" } else { "Reproduzir" };
        if add_icon_button(ui, &tex, btn_size, tooltip, ui.visuals().dark_mode) {
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
        "Anexar ao painel"
    } else {
        "Desacoplar vídeo"
    };
    if let Some(tex) = svg_manager.get_icon(ui.ctx(), detach_icon_name, 48, icon_color_val) {
        if add_icon_button(ui, &tex, btn_size, detach_tooltip, ui.visuals().dark_mode) {
            if is_detached && preview.is_maximized() {
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
            preview.toggle_detached();
        }
    }

    ui.add_space(2.0);

    // Volume Button (Mute/Unmute)
    // CRASH FIX: Use try-lock pattern to avoid deadlock with MPV thread
    let vol_icon = if is_muted { "vol_mute" } else { "vol_high" };
    if let Some(tex) = svg_manager.get_icon(ui.ctx(), vol_icon, 48, icon_color_val) {
        let vol_tooltip = if is_muted { "Ativar som" } else { "Mudo" };
        if add_icon_button(ui, &tex, btn_size, vol_tooltip, ui.visuals().dark_mode) {
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
    }
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

/// Draw audio track wheel picker
fn draw_audio_track_picker(ui: &mut egui::Ui, preview: &mut MediaPreview, _svg_manager: &mut SvgIconManager) {
    let audio_tracks = preview
        .get_video_state()
        .map(|s| s.audio_tracks.clone())
        .unwrap_or_default();

    if audio_tracks.is_empty() {
        return;
    }

    let current_idx = audio_tracks.iter().position(|t| t.selected).unwrap_or(0);
    let current_track = &audio_tracks[current_idx];
    let title = current_track.title.as_deref().unwrap_or("Audio");
    let lang = current_track.lang.as_deref().unwrap_or("unk");
    let full_text = format!("🎵 {} ({})", title, lang);

    if let Some(new_idx) = draw_wheel_picker(
        ui,
        &full_text,
        picker_width(),
        picker_height(),
        ui.visuals().dark_mode,
        current_idx,
        audio_tracks.len(),
    ) {
        if let Some(track) = audio_tracks.get(new_idx) {
            preview.set_audio_track(track.id);
        }
    }

    ui.add_space(4.0);
}

/// Draw subtitle wheel picker
fn draw_subtitle_track_picker(ui: &mut egui::Ui, preview: &mut MediaPreview, _svg_manager: &mut SvgIconManager) {
    let subtitle_tracks = preview
        .get_video_state()
        .map(|s| s.subtitle_tracks.clone())
        .unwrap_or_default();

    // Build options: [Off, Sub1, Sub2, ...]
    let mut sub_options: Vec<(Option<i64>, String)> = vec![(None, "CC: Off".to_string())];
    for track in &subtitle_tracks {
        let title = track.title.as_deref().unwrap_or("Legenda");
        let lang = track.lang.as_deref().unwrap_or("unk");
        sub_options.push((Some(track.id), format!("CC: {} ({})", title, lang)));
    }

    let current_sub_idx = subtitle_tracks
        .iter()
        .position(|t| t.selected)
        .map(|i| i + 1) // +1 because 0 is "Off"
        .unwrap_or(0);

    let full_sub_text = &sub_options[current_sub_idx].1;

    if let Some(new_idx) = draw_wheel_picker(
        ui,
        full_sub_text,
        picker_width(),
        picker_height(),
        ui.visuals().dark_mode,
        current_sub_idx,
        sub_options.len(),
    ) {
        let new_id = sub_options[new_idx].0.unwrap_or(0);
        preview.set_subtitle_track(new_id);
    }

    ui.add_space(2.0);
}

/// Standard picker dimensions
fn picker_width() -> f32 { 140.0 }
fn picker_height() -> f32 { 22.0 }

/// Generic wheel picker for tracks
fn draw_wheel_picker(
    ui: &mut egui::Ui,
    text: &str,
    width: f32,
    height: f32,
    dark_mode: bool,
    current_idx: usize,
    total_count: usize,
) -> Option<usize> {
    let font_id = egui::FontId::proportional(11.0);
    let display_text = truncate_text_to_fit(text, width - 16.0, &font_id, ui);
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(width, height),
        egui::Sense::click_and_drag(),
    );

    // Background with subtle border
    let bg_color = if dark_mode {
        egui::Color32::from_rgb(50, 50, 55)
    } else {
        egui::Color32::from_rgb(235, 235, 240)
    };
    let border_color = if response.hovered() {
        egui::Color32::from_rgb(100, 150, 200)
    } else if dark_mode {
        egui::Color32::from_rgb(70, 70, 75)
    } else {
        egui::Color32::from_rgb(200, 200, 205)
    };

    ui.painter().rect_filled(rect, 4.0, bg_color);
    ui.painter().rect_stroke(
        rect,
        4.0,
        egui::Stroke::new(1.0, border_color),
        egui::StrokeKind::Inside,
    );

    // Center text
    let text_color = if dark_mode {
        egui::Color32::from_gray(220)
    } else {
        egui::Color32::from_gray(40)
    };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        display_text,
        font_id,
        text_color,
    );

    // Handle scroll wheel
    let mut result = None;
    if response.hovered() {
        let scroll = ui.input(|i| i.raw_scroll_delta.y);
        if scroll != 0.0 {
            result = Some(if scroll > 0.0 {
                if current_idx > 0 {
                    current_idx - 1
                } else {
                    total_count - 1
                }
            } else {
                (current_idx + 1) % total_count
            });
        }
    }

    // Tooltip
    response.on_hover_text(format!(
        "{}/{} - Use scroll para trocar",
        current_idx + 1,
        total_count
    ));

    result
}

/// Draw audio normalizer button
fn draw_audio_normalizer(ui: &mut egui::Ui, preview: &mut MediaPreview, svg_manager: &mut SvgIconManager) {
    let normalizer_enabled = preview.is_audio_normalizer_enabled();
    let icon_color_val = icon_color(ui.visuals().dark_mode);
    let normalizer_color = if normalizer_enabled {
        [118, 185, 0, 255] // Green when enabled
    } else {
        icon_color_val
    };
    
    if let Some(tex) = svg_manager.get_icon(ui.ctx(), "headphones", 48, normalizer_color) {
        let tooltip = if normalizer_enabled {
            "Normalizador: Ativo"
        } else {
            "Normalizador: Inativo"
        };
        if add_icon_button(ui, &tex, 18.0, tooltip, ui.visuals().dark_mode) {
            preview.toggle_audio_normalizer();
        }
    } else {
        // Fallback: text button if icon doesn't load
        let label = if normalizer_enabled { "N+" } else { "N" };
        if ui.small_button(label).clicked() {
            preview.toggle_audio_normalizer();
        }
    }

    ui.add_space(4.0);
}

/// Draw detached-only buttons (fullscreen, VSR)
fn draw_detached_buttons(ui: &mut egui::Ui, preview: &mut MediaPreview, svg_manager: &mut SvgIconManager) {
    let icon_color_val = icon_color(ui.visuals().dark_mode);
    
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        // Fullscreen Button
        let is_fullscreen = preview.is_maximized();
        let fs_icon_name = if is_fullscreen {
            "minimize"
        } else {
            "maximize"
        };
        let fs_tooltip = if is_fullscreen {
            "Sair da Tela Cheia (ESC)"
        } else {
            "Tela Cheia"
        };

        if let Some(tex) = svg_manager.get_icon(ui.ctx(), fs_icon_name, 48, icon_color_val) {
            if add_icon_button(ui, &tex, 18.0, fs_tooltip, ui.visuals().dark_mode) {
                if !is_fullscreen {
                    // Entering fullscreen — only set flags here.
                    // The actual ViewportCommand::Fullscreen(true) is sent
                    // from render_fullscreen_video() on the next frame.
                    let was_maximized =
                        ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
                    preview.set_prev_app_maximized(was_maximized);
                    preview.set_fullscreen_applied(false);
                    preview.toggle_maximized();
                } else {
                    // Exiting fullscreen
                    preview.set_fullscreen_applied(false);
                    preview.set_forced_size(None); // Clear forced size when exiting fullscreen
                    preview.reset_last_rect(); // Force MPV window resize
                    preview.toggle_maximized();
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
                    if preview.prev_app_maximized() {
                        ui.ctx()
                            .send_viewport_cmd(egui::ViewportCommand::Maximized(true));
                    }
                }
            }
        }

        ui.add_space(4.0);

        // VSR Button (NVIDIA Video Super Resolution)
        let is_vsr = preview.is_vsr_enabled();
        let label = if is_vsr { "VSR On" } else { "VSR Off" };

        // Custom style for ON state (NVIDIA Green), Standard style for OFF state
        let btn = if is_vsr {
            egui::Button::new(
                egui::RichText::new(label)
                    .strong()
                    .size(10.0)
                    .color(egui::Color32::WHITE),
            )
            .fill(egui::Color32::from_rgb(118, 185, 0))
        } else {
            egui::Button::new(egui::RichText::new(label).size(10.0))
                .fill(egui::Color32::TRANSPARENT)
        };

        if ui
            .add(btn)
            .on_hover_text(if is_vsr {
                "Desativar NVIDIA VSR Upscaling"
            } else {
                "Ativar NVIDIA VSR (AI Upscaling)"
            })
            .clicked()
        {
            if let Err(e) = preview.toggle_vsr() {
                eprintln!("Error toggling VSR: {}", e);
            }
        }
    });
}

/// Draw simple controls for docked mode (preview panel)
/// Shows only: seek bar, play/pause, detach, volume, time
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
) {
    ui.set_width(full_width);

    // Seek Bar
    draw_seek_bar(ui, preview, full_width, current_time, duration);

    ui.add_space(6.0);

    // Buttons row - only basic controls for docked mode
    ui.horizontal(|ui| {
        // Basic controls (play, detach, volume)
        draw_basic_controls(
            ui,
            preview,
            svg_manager,
            is_playing,
            volume,
            is_muted,
            false, // is_detached = false
        );

        ui.add_space(10.0);

        // Time display
        draw_time_display(ui, current_time, duration);
    });
}

/// Draw full controls for detached mode (floating window)
/// Shows all controls including audio/subtitle pickers, normalizer, fullscreen, VSR
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
) {
    ui.set_width(full_width);

    // Seek Bar
    draw_seek_bar(ui, preview, full_width, current_time, duration);

    ui.add_space(6.0);

    // Buttons row - full controls for detached mode
    ui.horizontal(|ui| {
        // Basic controls (play, detach, volume)
        draw_basic_controls(
            ui,
            preview,
            svg_manager,
            is_playing,
            volume,
            is_muted,
            true, // is_detached = true
        );

        ui.add_space(10.0);

        // Time display
        draw_time_display(ui, current_time, duration);

        ui.add_space(8.0);

        // Audio track picker (only in detached mode)
        draw_audio_track_picker(ui, preview, svg_manager);

        // Subtitle track picker (only in detached mode)
        draw_subtitle_track_picker(ui, preview, svg_manager);

        // Audio normalizer (only in detached mode)
        draw_audio_normalizer(ui, preview, svg_manager);

        // Right-aligned buttons (fullscreen, VSR)
        draw_detached_buttons(ui, preview, svg_manager);
    });
}

/// Legacy function for backward compatibility - delegates to appropriate version
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
) {
    if is_detached {
        draw_detached_controls(
            ui, preview, full_width, svg_manager,
            is_playing, current_time, duration, volume, is_muted,
        );
    } else {
        draw_docked_controls(
            ui, preview, full_width, svg_manager,
            is_playing, current_time, duration, volume, is_muted,
        );
    }
}
