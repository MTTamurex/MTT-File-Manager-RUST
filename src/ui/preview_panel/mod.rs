pub mod actions;
pub mod fallback_renderer;
pub mod file_info_table;
pub mod image_preview;
pub mod utils;
pub mod video_preview;

use crate::domain::file_entry::FileEntry;
use crate::infrastructure::windows::MediaMetadata;
use crate::ui::components::MediaPreview;
use crate::ui::icon_loader::IconLoader;
use crate::ui::svg_icons::SvgIconManager;
use eframe::egui;

pub use actions::{PreviewPanelAction, PREVIEW_MAX_HEIGHT};
use fallback_renderer::render_fallback;
use file_info_table::render_file_info_table;
use image_preview::{render_gif_preview, render_texture_with_overlay};
use video_preview::render_video_preview;

pub fn render_preview_panel(
    ui: &mut egui::Ui,
    file: &FileEntry,
    multi_selection_count: usize,
    selected_thumbnail: Option<&egui::TextureHandle>,
    selected_gif: Option<&mut crate::ui::components::media_preview::GifPlayer>,
    media_preview: Option<&mut MediaPreview>,
    metadata: Option<&MediaMetadata>,
    texture_cache_peek: Option<egui::TextureHandle>, // Output of cache.peek
    folder_preview_peek: Option<egui::TextureHandle>, // Output of folder preview cache
    is_folder_preview_loading: bool,
    is_metadata_loading: bool,
    folder_size: Option<u64>,
    is_folder_size_loading: bool,
    is_recycle_bin_view: bool,
    item_icon_loader: &mut IconLoader,
    svg_manager: &mut SvgIconManager,
    frame: Option<&eframe::Frame>,
    is_owner: bool,
    is_failed: bool,
) -> Option<PreviewPanelAction> {
    // Metadados são processados de forma assíncrona; se chegarem, o metadata será Some(...)
    let mut action = None;

    // Check if this is a video file
    let is_video = file
        .path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| crate::infrastructure::windows::is_video_extension(ext))
        .unwrap_or(false);

    // === MULTI-SELECTION VIEW ===
    if multi_selection_count > 1 {
        ui.vertical_centered(|ui| {
            ui.add_space(40.0);

            // Multiple Items Icon (Stack)
            if let Some(tex) = svg_manager.get_icon(ui.ctx(), "copy", 128, [180, 180, 180, 255]) {
                ui.add(egui::Image::new(&tex).max_size(egui::vec2(128.0, 128.0)));
            } else {
                ui.label(egui::RichText::new("📚").size(64.0));
            }

            ui.add_space(20.0);
            ui.label(
                egui::RichText::new(format!("{} itens selecionados", multi_selection_count))
                    .strong()
                    .size(18.0),
            );
            ui.add_space(10.0);
            ui.label(
                egui::RichText::new("Selecione um único item para ver detalhes")
                    .color(egui::Color32::GRAY),
            );
        });
        return None;
    }

    ui.vertical_centered(|ui| {
        ui.add_space(20.0);

        // PERFORMANCE: Use is_media() method to avoid registry lookups
        let is_media = file.is_media();

        let texture = if is_failed || !is_media {
            None
        } else if let Some(tex) = selected_thumbnail {
            Some(tex.clone())
        } else {
            texture_cache_peek
        };

        if let Some(gif_player) = selected_gif {
            // === NATIVE GIF AUTOPLAY (PRIORITY 1) ===
            render_gif_preview(ui, gif_player);
        } else if let Some(preview) = media_preview {
            if is_video {
                // VIDEO PLAYER LOGIC (MPV)
                if let Some(act) = render_video_preview(
                    ui,
                    file,
                    preview,
                    svg_manager,
                    frame,
                    is_owner,
                    texture.as_ref(),
                ) {
                    action = Some(act);
                }
            } else {
                // === CURRENT FILE IS NOT A VIDEO (but media_preview exists for another file) ===
                // Show the image thumbnail, NOT the video player
                if let Some(tex) = &texture {
                    if let Some(act) = render_texture_with_overlay(ui, file, tex, svg_manager) {
                        action = Some(act);
                    }
                } else {
                    // Fallback for non-video items when video is present elsewhere
                    if let Some(act) = render_fallback(
                        ui,
                        file,
                        is_recycle_bin_view,
                        item_icon_loader,
                        svg_manager,
                        folder_preview_peek,
                        is_folder_preview_loading,
                    ) {
                        action = Some(act);
                    }
                }
            }
        } else if is_video {
            // === NO ACTIVE MEDIA PREVIEW YET (Non-owner tab or first selection) ===
            // Use video preview logic for thumbnail with play overlay
            // We need a way to call render_video_preview without a real preview struct?
            // Actually, render_video_preview needs MediaPreview.
            // Let's duplicate the thumbnail logic here or refactor.
            if let Some(tex) = &texture {
                let max_preview_width = ui.available_width() - 16.0;
                let max_preview_height = PREVIEW_MAX_HEIGHT;
                let max_preview_size = egui::vec2(max_preview_width, max_preview_height);

                let image_resp = ui.add(
                    egui::Image::new(tex)
                        .max_size(max_preview_size)
                        .shrink_to_fit(),
                );
                let media_rect = image_resp.rect;

                let hover_pos = ui.input(|i| i.pointer.hover_pos());
                let is_hovered = hover_pos.map_or(false, |pos| media_rect.contains(pos));

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
                if ui
                    .interact(
                        media_rect,
                        egui::Id::new("video_play_overlay_mod"),
                        egui::Sense::click(),
                    )
                    .clicked()
                {
                    action = Some(PreviewPanelAction::RequestPlay(file.path.clone()));
                }
            } else {
                ui.allocate_space(egui::vec2(ui.available_width() - 16.0, 200.0));
            }
        } else if let Some(tex) = &texture {
            // Fallback: Static Thumbnail (No MediaPreview state)
            if let Some(act) = render_texture_with_overlay(ui, file, tex, svg_manager) {
                action = Some(act);
            }
        } else {
            if let Some(act) = render_fallback(
                ui,
                file,
                is_recycle_bin_view,
                item_icon_loader,
                svg_manager,
                folder_preview_peek,
                is_folder_preview_loading,
            ) {
                action = Some(act);
            }
        }
        ui.add_space(20.0);
    });

    // Detail Table
    if let Some(act) = render_file_info_table(
        ui,
        file,
        metadata,
        folder_size,
        is_folder_size_loading,
        is_metadata_loading,
        svg_manager,
    ) {
        action = Some(act);
    }

    action
}
