use crate::domain::file_entry::{FileEntry, IconSize};
use crate::ui::icon_loader::IconLoader;
use crate::ui::preview_panel::actions::PreviewPanelAction;
use crate::ui::svg_icons::SvgIconManager;
use eframe::egui;

pub fn render_fallback(
    ui: &mut egui::Ui,
    file: &FileEntry,
    is_recycle_bin_view: bool,
    item_icon_loader: &mut IconLoader,
    svg_manager: &mut SvgIconManager,
    folder_preview_peek: Option<egui::TextureHandle>,
    is_folder_preview_loading: bool,
) -> Option<PreviewPanelAction> {
    let mut val_action = None;
    // Pasta ou Drive ou Arquivo sem Thumbnail
    let max_w: f32 = ui.available_width() - 40.0;
    let icon_size: f32 = (120.0f32).min(max_w);

    if file.name == "Este Computador" {
        // ESTE COMPUTADOR - usa o ícone de computador
        item_icon_loader.ensure_computer_icon(ui.ctx());
        if let Some(icon) = item_icon_loader.computer_icon() {
            ui.add(egui::Image::new(icon).max_size(egui::vec2(icon_size, icon_size)));
        } else {
            ui.label(egui::RichText::new("💻").size(icon_size * 0.6));
        }
    } else if let Some(_) = &file.drive_info {
        if let Some(icon) =
            item_icon_loader.get_or_load_drive_icon(ui.ctx(), &file.path.to_string_lossy())
        {
            ui.add(egui::Image::new(&icon).max_size(egui::vec2(icon_size, icon_size)));
        } else {
            ui.label(egui::RichText::new("??").size(icon_size * 0.8));
        }
    } else if is_recycle_bin_view && file.name == "Lixeira" {
        // LIXEIRA
        if let Some(icon) = item_icon_loader.ensure_recycle_bin_icon(ui.ctx()) {
            ui.add(egui::Image::new(&icon).max_size(egui::vec2(icon_size, icon_size)));
        } else {
            ui.label(egui::RichText::new("🗑").size(icon_size * 0.6));
        }
    } else if file.is_dir && !file.is_archive() {
        // PASTA (Exceto arquivos compactados)
        if is_recycle_bin_view {
            item_icon_loader.ensure_folder_icon(ui.ctx());
            if let Some(icon) = item_icon_loader.folder_icon() {
                ui.add(egui::Image::new(icon).max_size(egui::vec2(icon_size, icon_size)));
            } else {
                ui.label(egui::RichText::new("📁").size(icon_size * 0.6));
            }
        } else if crate::infrastructure::windows::shell_folder::is_shell_navigation_path(
            &file.path,
            file.is_dir,
        ) {
            // ZIP / SHELL PATH: Use System Folder Icon (No Preview)
            item_icon_loader.ensure_folder_icon(ui.ctx());
            if let Some(icon) = item_icon_loader.folder_icon() {
                ui.add(egui::Image::new(icon).max_size(egui::vec2(icon_size, icon_size)));
            } else {
                ui.label(egui::RichText::new("📁").size(icon_size * 0.6));
            }
        } else {
            let folder_rect = ui
                .allocate_exact_size(egui::vec2(icon_size, icon_size), egui::Sense::hover())
                .0;

            if let Some(tex) = folder_preview_peek.as_ref() {
                let tex_size = tex.size_vec2();
                let aspect = tex_size.x / tex_size.y;

                let (draw_w, draw_h) = if aspect > 1.0 {
                    (folder_rect.width(), folder_rect.width() / aspect)
                } else {
                    (folder_rect.height() * aspect, folder_rect.height())
                };

                let offset_x = (folder_rect.width() - draw_w) / 2.0;
                let offset_y = (folder_rect.height() - draw_h) / 2.0;
                let draw_rect = egui::Rect::from_min_size(
                    folder_rect.min + egui::vec2(offset_x, offset_y),
                    egui::vec2(draw_w, draw_h),
                );

                ui.painter().image(
                    tex.id(),
                    draw_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            } else if is_folder_preview_loading {
                // Spinner
                ui.painter()
                    .rect_filled(folder_rect, 4.0, egui::Color32::from_gray(245));
                ui.add(egui::Spinner::new());
            } else {
                // Dispara carregamento
                val_action = Some(PreviewPanelAction::LoadFolderPreview(file.path.clone()));

                // Placeholder
                ui.painter()
                    .rect_filled(folder_rect, 4.0, egui::Color32::from_gray(240));
                ui.painter().text(
                    folder_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "📁",
                    egui::FontId::proportional(icon_size * 0.4),
                    egui::Color32::from_gray(180),
                );
            }
        }
    } else {
        // IS FILE (or Archive)
        // Force is_folder=false for archives to get the archive icon
        let treat_as_folder = file.is_dir && !file.is_archive();

        if let Some(icon) = item_icon_loader.get_or_load_icon_sized(
            ui.ctx(),
            &file.path,
            IconSize::Jumbo,
            treat_as_folder,
            true,
        ) {
            let image_resp = ui.add(
                egui::Image::new(&icon).max_size(egui::vec2(icon_size * 0.8, icon_size * 0.8)),
            );

            // PDF Overlay for Fallback Icons
            let extension = file.path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if extension.eq_ignore_ascii_case("pdf") {
                let media_rect = image_resp.rect;
                let hover_pos = ui.input(|i| i.pointer.hover_pos());
                let is_hovered = hover_pos.map_or(false, |pos| media_rect.contains(pos));

                if is_hovered {
                    let center_size = 48.0;
                    let center_rect = egui::Rect::from_center_size(
                        media_rect.center(),
                        egui::vec2(center_size, center_size),
                    );

                    ui.painter().rect_filled(
                        center_rect,
                        center_size / 2.0,
                        egui::Color32::from_black_alpha(100),
                    );

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
                // Área de clique = todo o thumbnail
                if ui
                    .interact(
                        media_rect,
                        egui::Id::new("pdf_fallback_overlay"),
                        egui::Sense::click(),
                    )
                    .clicked()
                {
                    crate::pdf_viewer::open_pdf_viewer(file.path.clone());
                }
            }
        } else {
            ui.label(egui::RichText::new("??").size(icon_size * 0.6));
        }
    }
    val_action
}
