use crate::domain::file_entry::{FileEntry, IconSize};
use crate::domain::special_paths::{COMPUTER_VIEW_ID, RECYCLE_BIN_VIEW_ID};
use crate::ui::icon_loader::IconLoader;
use crate::ui::preview_panel::actions::PreviewPanelAction;
use crate::ui::svg_icons::SvgIconManager;
use eframe::egui;

/// Paints a texture centered within `container`, preserving aspect ratio.
fn paint_texture_centered(
    ui: &egui::Ui,
    tex_id: egui::TextureId,
    tex_size: egui::Vec2,
    container: egui::Rect,
) {
    let aspect = tex_size.x / tex_size.y;
    let container_aspect = container.width() / container.height();
    let (draw_w, draw_h) = if aspect > container_aspect {
        (container.width(), container.width() / aspect)
    } else {
        (container.height() * aspect, container.height())
    };
    let offset_x = (container.width() - draw_w) / 2.0;
    let offset_y = (container.height() - draw_h) / 2.0;
    let draw_rect = egui::Rect::from_min_size(
        container.min + egui::vec2(offset_x, offset_y),
        egui::vec2(draw_w, draw_h),
    );
    ui.painter().image(
        tex_id,
        draw_rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );
}

/// Paints a colored tag badge on the top-left corner of the icon rect.
fn paint_tag_badge(ui: &egui::Ui, icon_rect: egui::Rect, color: egui::Color32) {
    let center = egui::pos2(icon_rect.left() + 10.0, icon_rect.top() + 10.0);
    ui.painter().circle_filled(center, 6.0, color);
    ui.painter().circle_stroke(
        center,
        6.0,
        egui::Stroke::new(1.0, egui::Color32::from_black_alpha(80)),
    );
}

#[allow(clippy::too_many_arguments)]
pub fn render_fallback(
    ui: &mut egui::Ui,
    file: &FileEntry,
    is_recycle_bin_view: bool,
    item_icon_loader: &mut IconLoader,
    svg_manager: &mut SvgIconManager,
    folder_preview_peek: Option<egui::TextureHandle>,
    is_folder_preview_loading: bool,
    tag_color: Option<egui::Color32>,
) -> Option<PreviewPanelAction> {
    let mut val_action = None;
    // Folder, Drive, or File without Thumbnail
    let max_w: f32 = ui.available_width() - 40.0;
    let icon_size: f32 = (120.0f32).min(max_w);

    if file.name == COMPUTER_VIEW_ID {
        // THIS PC - uses the computer icon
        item_icon_loader.ensure_computer_icon(ui.ctx());
        if let Some(icon) = item_icon_loader.computer_icon() {
            ui.add(egui::Image::new(icon).max_size(egui::vec2(icon_size, icon_size)));
        } else {
            ui.allocate_response(egui::vec2(icon_size, icon_size), egui::Sense::hover());
        }
    } else if file.drive_info.is_some() {
        if let Some(icon) =
            item_icon_loader.get_or_load_drive_icon(ui.ctx(), &file.path.to_string_lossy())
        {
            ui.add(egui::Image::new(&icon).max_size(egui::vec2(icon_size, icon_size)));
        } else {
            ui.allocate_response(egui::vec2(icon_size, icon_size), egui::Sense::hover());
        }
    } else if is_recycle_bin_view && file.name == RECYCLE_BIN_VIEW_ID {
        // RECYCLE BIN
        if let Some(icon) = item_icon_loader.ensure_recycle_bin_icon(ui.ctx()) {
            ui.add(egui::Image::new(&icon).max_size(egui::vec2(icon_size, icon_size)));
        } else {
            ui.allocate_response(egui::vec2(icon_size, icon_size), egui::Sense::hover());
        }
    } else if file.is_dir && !file.is_archive() {
        // FOLDER (Except compressed files)

        // Detect system paths (C:\Windows tree) — always use static folder icon,
        // no async preview, no spinner, no placeholder.
        let is_system_path =
            crate::infrastructure::windows::is_windows_system_path(&file.path.to_string_lossy());

        if is_recycle_bin_view
            || is_system_path
            || crate::infrastructure::windows::shell_folder::is_shell_navigation_path(
                &file.path,
                file.is_dir,
            )
        {
            // Recycle bin, system path, or shell navigation: simple folder icon
            let folder_rect = ui
                .allocate_exact_size(egui::vec2(icon_size, icon_size), egui::Sense::hover())
                .0;
            if let Some(icon) = item_icon_loader.folder_icon() {
                paint_texture_centered(ui, icon.id(), icon.size_vec2(), folder_rect);
            }
            if let Some(color) = tag_color {
                paint_tag_badge(ui, folder_rect, color);
            }
        } else if item_icon_loader.has_registered_folder_icon(&file.path.to_string_lossy())
            || crate::infrastructure::onedrive::is_special_icon_folder(&file.path)
        {
            // Special folder (Documents, Pictures, Desktop, etc.) — use native shell icon
            let folder_rect = ui
                .allocate_exact_size(egui::vec2(icon_size, icon_size), egui::Sense::hover())
                .0;
            let icon = item_icon_loader
                .get_or_load_registered_folder_icon(ui.ctx(), &file.path.to_string_lossy())
                .or_else(|| {
                    item_icon_loader
                        .get_or_load_folder_path_icon(ui.ctx(), &file.path.to_string_lossy())
                        .or_else(|| item_icon_loader.folder_icon().cloned())
                });
            if let Some(icon) = icon {
                paint_texture_centered(ui, icon.id(), icon.size_vec2(), folder_rect);
            }
            if let Some(color) = tag_color {
                paint_tag_badge(ui, folder_rect, color);
            }
        } else {
            let folder_rect = ui
                .allocate_exact_size(egui::vec2(icon_size, icon_size), egui::Sense::hover())
                .0;

            if let Some(tex) = folder_preview_peek.as_ref() {
                paint_texture_centered(ui, tex.id(), tex.size_vec2(), folder_rect);
            } else {
                if !is_folder_preview_loading {
                    // Trigger loading
                    val_action = Some(PreviewPanelAction::LoadFolderPreview(file.path.clone()));
                }
                // Show folder icon while preview loads or triggers
                if let Some(icon) = item_icon_loader.folder_icon() {
                    paint_texture_centered(ui, icon.id(), icon.size_vec2(), folder_rect);
                }
            }
            if let Some(color) = tag_color {
                paint_tag_badge(ui, folder_rect, color);
            }
        }
    } else {
        // IS FILE (or Archive)
        // Force is_folder=false for archives to get the archive icon
        let treat_as_folder = file.is_dir && !file.is_archive();
        let is_virtual_archive_path = crate::domain::file_entry::is_path_inside_archive(&file.path);

        // First try non-blocking: returns cached Jumbo icon if available.
        let icon = item_icon_loader
            .get_or_load_icon_sized(
                ui.ctx(),
                &file.path,
                IconSize::Jumbo,
                treat_as_folder,
                false, // never block the UI thread
            )
            .or_else(|| {
                // Jumbo not cached yet — trigger async extraction and show Large
                // fallback in the meantime.
                item_icon_loader.enqueue_jumbo_icon(&file.path, is_virtual_archive_path);
                // Try Large as immediate fallback.
                item_icon_loader.get_or_load_icon_sized(
                    ui.ctx(),
                    &file.path,
                    IconSize::Large,
                    treat_as_folder,
                    false,
                )
            });

        if let Some(icon) = icon {
            let image_resp = ui.add(
                egui::Image::new(&icon).max_size(egui::vec2(icon_size * 0.8, icon_size * 0.8)),
            );

            // PDF/Image overlay for fallback icons
            let extension = file.path.extension().and_then(|e| e.to_str()).unwrap_or("");
            let is_pdf = extension.eq_ignore_ascii_case("pdf");
            let is_image = crate::infrastructure::windows::is_image_extension(extension);
            let is_text = crate::text_viewer::is_text_extension(extension);
            if !is_virtual_archive_path && (is_pdf || is_image || is_text) {
                let media_rect = image_resp.rect;
                let hover_pos = ui.input(|i| i.pointer.hover_pos());
                let is_hovered = hover_pos.is_some_and(|pos| media_rect.contains(pos));

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
                // Click area = entire thumbnail
                if ui
                    .interact(
                        media_rect,
                        egui::Id::new(if is_pdf {
                            "pdf_fallback_overlay"
                        } else {
                            "image_fallback_overlay"
                        }),
                        egui::Sense::click(),
                    )
                    .clicked()
                {
                    if is_pdf {
                        crate::pdf_viewer::open_pdf_viewer(file.path.clone());
                    } else if is_image {
                        crate::image_viewer::open_image_viewer(file.path.clone());
                    } else if is_text {
                        crate::text_viewer::open_text_viewer(file.path.clone());
                    }
                }
            }
        } else {
            ui.allocate_response(
                egui::vec2(icon_size * 0.8, icon_size * 0.8),
                egui::Sense::hover(),
            );
        }
    }
    val_action
}
