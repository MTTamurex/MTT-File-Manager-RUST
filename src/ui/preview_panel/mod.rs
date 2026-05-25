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
use video_preview::{render_media_launcher, render_video_preview};

#[allow(clippy::too_many_arguments)]
pub fn render_preview_panel(
    ui: &mut egui::Ui,
    file: &FileEntry,
    multi_selection_count: usize,
    multi_selection_total_size: u64,
    selected_thumbnail: Option<&egui::TextureHandle>,
    selected_gif: Option<&mut crate::ui::components::media_preview::GifPlayer>,
    media_preview: Option<&mut MediaPreview>,
    metadata: Option<&MediaMetadata>,
    texture_cache_peek: Option<egui::TextureHandle>, // Output of cache.peek
    folder_preview_peek: Option<egui::TextureHandle>, // Output of folder preview cache
    is_folder_preview_loading: bool,
    is_metadata_loading: bool,
    folder_summary: Option<crate::app::folder_size_state::FolderContentSummary>,
    is_folder_size_loading: bool,
    live_file_size_cache: &mut lru::LruCache<std::path::PathBuf, (u64, u64)>,
    live_file_size_loading: &mut crate::ui::cache::FxHashSet<std::path::PathBuf>,
    live_file_size_req_sender: &std::sync::mpsc::Sender<
        crate::app::live_file_size::LiveFileSizeRequest,
    >,
    is_recycle_bin_view: bool,
    item_icon_loader: &mut IconLoader,
    svg_manager: &mut SvgIconManager,
    frame: Option<&eframe::Frame>,
    is_owner: bool,
    is_failed: bool,
) -> Option<PreviewPanelAction> {
    // Metadata is processed asynchronously; when it arrives, metadata will be Some(...)
    let mut action = None;

    // Check if this is a playable media file for MPV
    let extension = file
        .path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("");
    let is_video = crate::infrastructure::windows::is_video_extension(extension);
    let is_audio = crate::infrastructure::windows::is_audio_extension(extension);
    let is_playable_media = is_video || is_audio;

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
                egui::RichText::new(
                    rust_i18n::t!("preview.items_selected", count = multi_selection_count)
                        .to_string(),
                )
                .strong()
                .size(18.0),
            );
            ui.add_space(6.0);
            let total_size_text =
                crate::infrastructure::windows::format_size(multi_selection_total_size);
            ui.label(
                egui::RichText::new(
                    rust_i18n::t!("preview.total_size", size = total_size_text).to_string(),
                )
                .size(14.0),
            );
            ui.add_space(10.0);
            ui.label(
                egui::RichText::new(rust_i18n::t!("preview.select_item").to_string())
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
            render_gif_preview(ui, file, gif_player, texture.as_ref(), svg_manager);
        } else if let Some(preview) = media_preview {
            if is_playable_media {
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
        } else if is_playable_media {
            // === NO ACTIVE MEDIA PREVIEW YET (Non-owner tab or first selection) ===
            if let Some(act) = render_media_launcher(ui, file, svg_manager, texture.as_ref()) {
                action = Some(act);
            }
        } else if let Some(tex) = &texture {
            // Fallback: Static Thumbnail (No MediaPreview state)
            if let Some(act) = render_texture_with_overlay(ui, file, tex, svg_manager) {
                action = Some(act);
            }
        } else if let Some(act) = render_fallback(
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
        ui.add_space(20.0);
    });

    // Detail Table
    if let Some(act) = render_file_info_table(
        ui,
        file,
        metadata,
        folder_summary,
        is_folder_size_loading,
        is_metadata_loading,
        live_file_size_cache,
        live_file_size_loading,
        live_file_size_req_sender,
        svg_manager,
    ) {
        action = Some(act);
    }

    action
}
