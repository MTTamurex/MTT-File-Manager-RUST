use crate::domain::file_entry::FileEntry;
use crate::ui::preview_panel::actions::{PreviewPanelAction, PREVIEW_MAX_HEIGHT};
use crate::ui::svg_icons::SvgIconManager;
use eframe::egui;

pub fn render_texture_with_overlay(
    ui: &mut egui::Ui,
    file: &FileEntry,
    tex: &egui::TextureHandle,
    svg_manager: &mut SvgIconManager,
) -> Option<PreviewPanelAction> {
    let max_preview_width = ui.available_width() - 16.0;
    let max_preview_height = PREVIEW_MAX_HEIGHT;
    let max_preview_size = egui::vec2(max_preview_width, max_preview_height);

    let image_resp = ui.add(
        egui::Image::new(tex)
            .max_size(max_preview_size)
            .shrink_to_fit(),
    );

    let extension = file.path.extension().and_then(|e| e.to_str()).unwrap_or("");

    let media_rect = image_resp.rect;
    let hover_pos = ui.input(|i| i.pointer.hover_pos());
    let is_hovered = hover_pos.map_or(false, |pos| media_rect.contains(pos));

    if is_hovered {
        if extension.eq_ignore_ascii_case("pdf") {
            let center_size = 48.0;
            let center_rect = egui::Rect::from_center_size(
                media_rect.center(),
                egui::vec2(center_size, center_size),
            );

            // Draw background for contrast
            ui.painter().rect_filled(
                center_rect,
                center_size / 2.0,
                egui::Color32::from_black_alpha(100),
            );

            // Draw Lupa (Search) Icon
            if let Some(tex_lupa) =
                svg_manager.get_icon(ui.ctx(), "search", 96, [255, 255, 255, 255])
            {
                ui.painter().image(
                    tex_lupa.id(),
                    center_rect.shrink(10.0),
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            } else {
                ui.painter().text(
                    center_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "🔍",
                    egui::FontId::proportional(24.0),
                    egui::Color32::WHITE,
                );
            }
        } else if crate::infrastructure::windows::is_image_extension(extension) {
            let center_size = 48.0;
            let center_rect = egui::Rect::from_center_size(
                media_rect.center(),
                egui::vec2(center_size, center_size),
            );

            // Draw background for contrast
            ui.painter().rect_filled(
                center_rect,
                center_size / 2.0,
                egui::Color32::from_black_alpha(100),
            );

            // Draw Lupa (Search) Icon
            if let Some(tex_lupa) =
                svg_manager.get_icon(ui.ctx(), "search", 96, [255, 255, 255, 255])
            {
                ui.painter().image(
                    tex_lupa.id(),
                    center_rect.shrink(10.0),
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            } else {
                ui.painter().text(
                    center_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "🔍",
                    egui::FontId::proportional(24.0),
                    egui::Color32::WHITE,
                );
            }
        }
    }

    // Área de clique = todo o thumbnail (PDF ou imagem)
    if extension.eq_ignore_ascii_case("pdf") {
        if ui
            .interact(
                media_rect,
                egui::Id::new("pdf_thumb_overlay"),
                egui::Sense::click(),
            )
            .clicked()
        {
            crate::pdf_viewer::open_pdf_viewer(file.path.clone());
        }
    } else if crate::infrastructure::windows::is_image_extension(extension) {
        if ui
            .interact(
                media_rect,
                egui::Id::new("image_thumb_overlay"),
                egui::Sense::click(),
            )
            .clicked()
        {
            crate::pdf_viewer::open_image_viewer(file.path.clone());
        }
    }

    None
}

pub fn render_gif_preview(
    ui: &mut egui::Ui,
    gif_player: &mut crate::ui::components::media_preview::GifPlayer,
) {
    gif_player.update(ui.ctx());
    if let Some(texture) = gif_player.texture() {
        let max_preview_width = ui.available_width() - 16.0;
        let max_preview_height = PREVIEW_MAX_HEIGHT;
        let max_preview_size = egui::vec2(max_preview_width, max_preview_height);
        ui.add(
            egui::Image::new(texture)
                .max_size(max_preview_size)
                .shrink_to_fit(),
        );
    } else {
        ui.add(egui::Spinner::new());
    }
}
