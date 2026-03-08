use super::{add_icon_button, icon_color};
use crate::ui::components::MediaPreview;
use crate::ui::preview_panel::utils::truncate_text_to_fit;
use crate::ui::svg_icons::SvgIconManager;
use eframe::egui;
use rfd::FileDialog;
use rust_i18n::t;

/// Draw audio track wheel picker
pub(super) fn draw_audio_track_picker(ui: &mut egui::Ui, preview: &mut MediaPreview) {
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
pub(super) fn draw_subtitle_track_picker(
    ui: &mut egui::Ui,
    preview: &mut MediaPreview,
    svg_manager: &mut SvgIconManager,
) {
    let subtitle_tracks = preview
        .get_video_state()
        .map(|s| s.subtitle_tracks.clone())
        .unwrap_or_default();

    // Build options: [Off, Sub1, Sub2, ...]
    let mut sub_options: Vec<(Option<i64>, String)> = vec![(None, "CC: Off".to_string())];
    for track in &subtitle_tracks {
        let fallback = t!("video.subtitle_fallback_label");
        let title = track.title.as_deref().unwrap_or(&fallback);
        let lang = track.lang.as_deref().unwrap_or("unk");
        sub_options.push((Some(track.id), format!("CC: {} ({})", title, lang)));
    }

    let current_sub_idx = subtitle_tracks
        .iter()
        .position(|t| t.selected)
        .map(|i| i + 1)
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

    ui.add_space(4.0);

    let icon_color_val = icon_color(ui.visuals().dark_mode);
    if let Some(tex) = svg_manager.get_icon(ui.ctx(), "languages", 48, icon_color_val) {
        if add_icon_button(
            ui,
            &tex,
            18.0,
            &t!("video.load_external_subtitle"),
            ui.visuals().dark_mode,
        ) {
            let mut file_dialog =
                FileDialog::new().add_filter(t!("video.subtitle_filter").to_string(), &["srt", "ass", "ssa", "vtt", "sub"]);

            if let Some(current_video_path) = preview.path() {
                if let Some(parent) = current_video_path.parent() {
                    file_dialog = file_dialog.set_directory(parent);
                }
            }

            if let Some(subtitle_path) = file_dialog.pick_file() {
                if let Err(e) = preview.load_external_subtitle(&subtitle_path) {
                    log::error!("[MPV] Failed to load subtitle from picker: {}", e);
                }
            }
        }
    }

    ui.add_space(2.0);
}

/// Standard picker dimensions
fn picker_width() -> f32 {
    140.0
}

fn picker_height() -> f32 {
    22.0
}

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
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click_and_drag());

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
