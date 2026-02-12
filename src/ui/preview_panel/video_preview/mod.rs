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
    let max_preview_height = PREVIEW_MAX_HEIGHT;
    let max_preview_size = egui::vec2(max_preview_width, max_preview_height);

    // PATH CHECK: Only show active player if the file is the one playing AND we are the owner
    let paths_match = preview.path() == Some(&file.path);

    if paths_match && is_owner {
        // === ACTIVE PLAYER (OWNER) ===
        if preview.is_maximized() {
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
            );
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
            );
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
            );
        }
    } else {
        // === THUMBNAIL WITH PLAY OVERLAY (NON-OWNER) ===
        if let Some(tex) = texture {
            let image_resp = ui.add(
                egui::Image::new(tex)
                    .max_size(max_preview_size)
                    .shrink_to_fit(),
            );
            let media_rect = image_resp.rect;

            // Central play button on hover
            let hover_pos = ui.input(|i| i.pointer.hover_pos());
            let is_hovered = hover_pos.is_some_and(|pos| media_rect.contains(pos));

            if is_hovered {
                let center_size = 64.0;
                let center_rect = egui::Rect::from_center_size(
                    media_rect.center(),
                    egui::vec2(center_size, center_size),
                );
                ui.painter().rect_filled(
                    center_rect,
                    center_size / 2.0,
                    egui::Color32::from_black_alpha(160),
                );
                if let Some(tex_play) =
                    svg_manager.get_icon(ui.ctx(), "play", 96, [255, 255, 255, 255])
                {
                    ui.painter().image(
                        tex_play.id(),
                        center_rect.shrink(14.0),
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                }
            }
            // Click area = entire thumbnail
            if ui
                .interact(
                    media_rect,
                    egui::Id::new("video_play_overlay_router"),
                    egui::Sense::click(),
                )
                .clicked()
            {
                action = Some(PreviewPanelAction::RequestPlay(file.path.clone()));
            }
        } else {
            ui.allocate_space(egui::vec2(max_preview_width, 200.0));
        }
    }
    action
}
