pub mod controls;
pub mod detached;
pub mod docked;
pub mod fullscreen;

use crate::domain::file_entry::FileEntry;
use crate::ui::components::MediaPreview;
use crate::ui::preview_panel::actions::{PreviewPanelAction, PREVIEW_MAX_HEIGHT};
use crate::ui::preview_panel::video_preview::detached::render_detached_video;
use crate::ui::preview_panel::video_preview::docked::render_docked_video;
use crate::ui::preview_panel::video_preview::fullscreen::render_fullscreen_video;
use crate::ui::svg_icons::SvgIconManager;
use eframe::egui;

pub fn render_media_launcher(
    ui: &mut egui::Ui,
    file: &FileEntry,
    svg_manager: &mut SvgIconManager,
    texture: Option<&egui::TextureHandle>,
) -> Option<PreviewPanelAction> {
    let preview_launch_allowed = !crate::domain::file_entry::is_path_inside_archive(&file.path);
    let max_preview_width = ui.available_width() - 16.0;
    let max_preview_height = PREVIEW_MAX_HEIGHT;
    let max_preview_size = egui::vec2(max_preview_width, max_preview_height);

    let media_rect = if let Some(tex) = texture {
        ui.add(
            egui::Image::new(tex)
                .max_size(max_preview_size)
                .shrink_to_fit(),
        )
        .rect
    } else {
        let desired_size = egui::vec2(max_preview_width.max(120.0), max_preview_height);
        let (rect, _) = ui.allocate_exact_size(desired_size, egui::Sense::hover());

        let visuals = ui.visuals();
        let fill = if visuals.dark_mode {
            egui::Color32::from_rgb(28, 28, 32)
        } else {
            egui::Color32::from_rgb(243, 245, 248)
        };
        let stroke = if visuals.dark_mode {
            egui::Color32::from_rgb(58, 58, 64)
        } else {
            egui::Color32::from_rgb(216, 220, 226)
        };
        ui.painter().rect(
            rect,
            16.0,
            fill,
            egui::Stroke::new(1.0, stroke),
            egui::StrokeKind::Outside,
        );

        let is_audio = file
            .path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(crate::infrastructure::windows::is_audio_extension)
            .unwrap_or(false);
        let icon_name = if is_audio { "headphones" } else { "play" };
        let icon_color = if visuals.dark_mode {
            [214, 218, 224, 255]
        } else {
            [96, 102, 110, 255]
        };
        if let Some(icon) = svg_manager.get_icon(ui.ctx(), icon_name, 128, icon_color) {
            let icon_rect = egui::Rect::from_center_size(rect.center(), egui::vec2(72.0, 72.0));
            ui.painter().image(
                icon.id(),
                icon_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }

        rect
    };

    let hover_pos = ui.input(|i| i.pointer.hover_pos());
    let is_hovered = hover_pos.is_some_and(|pos| media_rect.contains(pos));

    if preview_launch_allowed && is_hovered {
        let center_size = 64.0;
        let center_rect =
            egui::Rect::from_center_size(media_rect.center(), egui::vec2(center_size, center_size));
        ui.painter().rect_filled(
            center_rect,
            center_size / 2.0,
            egui::Color32::from_black_alpha(160),
        );
        if let Some(tex_play) = svg_manager.get_icon(ui.ctx(), "play", 96, [255, 255, 255, 255]) {
            ui.painter().image(
                tex_play.id(),
                center_rect.shrink(14.0),
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
    }

    if preview_launch_allowed
        && ui
            .interact(
                media_rect,
                egui::Id::new(("media_play_overlay", &file.path)),
                egui::Sense::click(),
            )
            .clicked()
    {
        return Some(PreviewPanelAction::RequestPlay(file.path.clone()));
    }

    None
}

pub fn render_video_preview(
    ui: &mut egui::Ui,
    file: &FileEntry,
    preview: &mut MediaPreview,
    svg_manager: &mut SvgIconManager,
    frame: Option<&eframe::Frame>,
    is_owner: bool,
    texture: Option<&egui::TextureHandle>,
) -> Option<PreviewPanelAction> {
    let mut action = None;
    let video_state = preview.get_video_state();
    let is_playing = video_state.as_ref().map(|s| s.is_playing).unwrap_or(false);
    let current_time = video_state.as_ref().map(|s| s.current_time).unwrap_or(0.0);
    let duration = video_state.as_ref().map(|s| s.duration).unwrap_or(0.0);
    let volume = video_state.as_ref().map(|s| s.volume).unwrap_or(1.0);
    let is_muted = video_state.as_ref().map(|s| s.is_muted).unwrap_or(false);

    let max_preview_width = ui.available_width() - 16.0;

    // PATH CHECK: Only show active player if the file is the one playing AND we are the owner
    let paths_match = preview.path() == Some(&file.path);

    if paths_match && is_owner {
        // === ACTIVE PLAYER (OWNER) ===
        let vol_action = if preview.is_maximized() {
            render_fullscreen_video(
                ui,
                preview,
                svg_manager,
                frame,
                current_time,
                duration,
                volume,
                is_muted,
                is_playing,
            )
        } else if preview.is_detached() {
            render_detached_video(
                ui,
                preview,
                svg_manager,
                frame,
                &file.name,
                current_time,
                duration,
                volume,
                is_muted,
                is_playing,
            )
        } else {
            render_docked_video(
                ui,
                preview,
                svg_manager,
                frame,
                max_preview_width,
                current_time,
                duration,
                volume,
                is_muted,
                is_playing,
            )
        };
        action = action.or(vol_action);
    } else {
        // === THUMBNAIL WITH PLAY OVERLAY (NON-OWNER) ===
        if let Some(launcher_action) = render_media_launcher(ui, file, svg_manager, texture) {
            action = Some(launcher_action);
        }
    }
    action
}
