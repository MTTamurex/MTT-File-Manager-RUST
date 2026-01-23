use crate::domain::file_entry::FileEntry;
use crate::infrastructure::windows::MediaMetadata;
use crate::ui::components::MediaPreview;
use crate::ui::icon_loader::IconLoader;
use crate::ui::svg_icons::SvgIconManager;
use eframe::egui;
use std::path::PathBuf;

pub enum PreviewPanelAction {
    RefreshThumbnail(PathBuf),
    LoadFolderPreview(PathBuf),
    CalculateFolderSize(PathBuf),
    RequestPlay(PathBuf),
}

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
    let is_video = file.path.extension()
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
            ui.label(egui::RichText::new(format!("{} itens selecionados", multi_selection_count)).strong().size(18.0));
            ui.add_space(10.0);
            ui.label(egui::RichText::new("Selecione um único item para ver detalhes").color(egui::Color32::GRAY));
        });
        return None;
    }

    // Reuseable fallback logic for rendering icons when no preview is available
    let mut render_fallback = |ui: &mut egui::Ui, svg_manager: &mut SvgIconManager| -> Option<PreviewPanelAction> {
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

            } else if file.is_dir && !file.name.to_lowercase().ends_with(".zip") {
                // PASTA (Exceto Zip)
                if is_recycle_bin_view {
                    item_icon_loader.ensure_folder_icon(ui.ctx());
                    if let Some(icon) = item_icon_loader.folder_icon() {
                        ui.add(egui::Image::new(icon).max_size(egui::vec2(icon_size, icon_size)));
                    } else {
                        ui.label(egui::RichText::new("📁").size(icon_size * 0.6));
                    }
                } else if crate::infrastructure::windows::shell_folder::is_shell_navigation_path(&file.path) {
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
                        // ... (Render Preview as before)
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
                // IS FILE (or Zip Archive)
                use crate::domain::file_entry::IconSize;
                // Force is_folder=false for Zip archives to get the Zip Icon
                let is_zip_archive = file.name.to_lowercase().ends_with(".zip");
                let treat_as_folder = file.is_dir && !is_zip_archive;
                
                if let Some(icon) = item_icon_loader.get_or_load_icon_sized(ui.ctx(), &file.path, IconSize::Jumbo, treat_as_folder) {
                    let image_resp = ui.add(
                        egui::Image::new(&icon)
                            .max_size(egui::vec2(icon_size * 0.8, icon_size * 0.8)),
                    );

                    // PDF Overlay for Fallback Icons
                    let extension = file.path.extension().and_then(|e| e.to_str()).unwrap_or("");
                    if extension.eq_ignore_ascii_case("pdf") {
                        let media_rect = image_resp.rect;
                        let hover_pos = ui.input(|i| i.pointer.hover_pos());
                        let is_hovered = hover_pos.map_or(false, |pos| media_rect.contains(pos));

                        if is_hovered {
                            let center_size = 48.0;
                            let center_rect = egui::Rect::from_center_size(media_rect.center(), egui::vec2(center_size, center_size));

                            ui.painter().rect_filled(center_rect, center_size / 2.0, egui::Color32::from_black_alpha(100));

                            if let Some(tex_lupa) = svg_manager.get_icon(ui.ctx(), "search", 96, [255, 255, 255, 255]) {
                                 ui.painter().image(tex_lupa.id(), center_rect.shrink(10.0), egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
                            } else {
                                 ui.painter().text(center_rect.center(), egui::Align2::CENTER_CENTER, "🔍", egui::FontId::proportional(24.0), egui::Color32::WHITE);
                            }
                        }
                        // Área de clique = todo o thumbnail
                        if ui.interact(media_rect, egui::Id::new("pdf_fallback_overlay"), egui::Sense::click()).clicked() {
                            crate::pdf_viewer::open_pdf_viewer(file.path.clone());
                        }
                    }
                } else {
                    ui.label(egui::RichText::new("??").size(icon_size * 0.6));
                }
            }
            val_action
    };

    // Shared logic for rendering thumbnails with overlays (PDF, Image Magnifier)
    let render_texture_with_overlay = |ui: &mut egui::Ui, tex: &egui::TextureHandle, svg_manager: &mut SvgIconManager| -> Option<PreviewPanelAction> {
        let max_preview_width = ui.available_width() - 16.0;
        let max_preview_size = egui::vec2(max_preview_width, max_preview_width);

        let image_resp = ui.add(
            egui::Image::new(tex)
                .max_size(max_preview_size)
                .shrink_to_fit(),
        );

        let local_action = None;
        let extension = file.path.extension().and_then(|e| e.to_str()).unwrap_or("");

        let media_rect = image_resp.rect;
        let hover_pos = ui.input(|i| i.pointer.hover_pos());
        let is_hovered = hover_pos.map_or(false, |pos| media_rect.contains(pos));

        if is_hovered {
             if extension.eq_ignore_ascii_case("pdf") {
                let center_size = 48.0;
                let center_rect = egui::Rect::from_center_size(media_rect.center(), egui::vec2(center_size, center_size));

                // Draw background for contrast
                ui.painter().rect_filled(center_rect, center_size / 2.0, egui::Color32::from_black_alpha(100));

                // Draw Lupa (Search) Icon
                if let Some(tex_lupa) = svg_manager.get_icon(ui.ctx(), "search", 96, [255, 255, 255, 255]) {
                        ui.painter().image(tex_lupa.id(), center_rect.shrink(10.0), egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
                } else {
                        ui.painter().text(center_rect.center(), egui::Align2::CENTER_CENTER, "🔍", egui::FontId::proportional(24.0), egui::Color32::WHITE);
                }
            } else if crate::infrastructure::windows::is_image_extension(extension) {
                let center_size = 48.0;
                let center_rect = egui::Rect::from_center_size(media_rect.center(), egui::vec2(center_size, center_size));

                // Draw background for contrast
                ui.painter().rect_filled(center_rect, center_size / 2.0, egui::Color32::from_black_alpha(100));

                // Draw Lupa (Search) Icon
                if let Some(tex_lupa) = svg_manager.get_icon(ui.ctx(), "search", 96, [255, 255, 255, 255]) {
                        ui.painter().image(tex_lupa.id(), center_rect.shrink(10.0), egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
                } else {
                        ui.painter().text(center_rect.center(), egui::Align2::CENTER_CENTER, "🔍", egui::FontId::proportional(24.0), egui::Color32::WHITE);
                }
            }
        }

        // Área de clique = todo o thumbnail (PDF ou imagem)
        if extension.eq_ignore_ascii_case("pdf") {
            if ui.interact(media_rect, egui::Id::new("pdf_thumb_overlay"), egui::Sense::click()).clicked() {
                crate::pdf_viewer::open_pdf_viewer(file.path.clone());
            }
        } else if crate::infrastructure::windows::is_image_extension(extension) {
            if ui.interact(media_rect, egui::Id::new("image_thumb_overlay"), egui::Sense::click()).clicked() {
                crate::pdf_viewer::open_image_viewer(file.path.clone());
            }
        }

        local_action
    };

    ui.vertical_centered(|ui| {
        ui.add_space(20.0);

        // Only show thumbnails for images, videos or audio files.
        // Other files (.rar, .7z, .pdf, etc.) will use high-quality Jumbo icons in the fallback.
        let extension = file.path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let is_media = crate::infrastructure::windows::is_media_extension(extension);

        let texture = if is_failed || !is_media {
            None
        } else if let Some(tex) = selected_thumbnail {
            Some(tex.clone())
        } else {
            texture_cache_peek
        };

        if let Some(gif_player) = selected_gif {
            // === NATIVE GIF AUTOPLAY (PRIORITY 1) ===
            gif_player.update(ui.ctx());
            if let Some(texture) = gif_player.texture() {
                let max_preview_width = ui.available_width() - 16.0;
                let max_preview_size = egui::vec2(max_preview_width, max_preview_width);
                ui.add(egui::Image::new(texture).max_size(max_preview_size).shrink_to_fit());
            } else {
                ui.add(egui::Spinner::new());
            }
        } else if let Some(preview) = media_preview {
            if is_video {
                // VIDEO PLAYER LOGIC (MPV)
                let is_player_visible = preview.is_player_visible();
                let video_state = preview.get_video_state();
                let is_playing = video_state.as_ref().map(|s| s.is_playing).unwrap_or(false);
                let current_time = video_state.as_ref().map(|s| s.current_time).unwrap_or(0.0);
                let duration = video_state.as_ref().map(|s| s.duration).unwrap_or(0.0);
                let volume = video_state.as_ref().map(|s| s.volume).unwrap_or(1.0);
                let is_muted = video_state.as_ref().map(|s| s.is_muted).unwrap_or(false);

                let max_preview_width = ui.available_width() - 16.0;
                let max_preview_size = egui::vec2(max_preview_width, max_preview_width);

                // PATH CHECK: Only show active player if the file is the one playing AND we are the owner
                let paths_match = preview.path() == Some(&file.path);
                
                if is_player_visible && paths_match && is_owner {
                    // === ACTIVE PLAYER (OWNER) ===
                    let is_detached = preview.is_detached();

                    // Control Builder Closure (Shared between Attached and Detached views)
                    // NOW TAKES preview AND svg_manager AS ARGUMENT TO AVOID BORROW ISSUES
                    let draw_controls = |ui: &mut egui::Ui, preview: &mut MediaPreview, full_width: f32, svg_manager: &mut SvgIconManager| {
                         ui.set_width(full_width);
                        
                        // Seek Bar
                        ui.horizontal(|ui| {
                            ui.spacing_mut().slider_width = full_width;
                            ui.visuals_mut().selection.bg_fill = crate::ui::theme::COLOR_ACCENT;

                            let mut seek_value = current_time;
                            if ui.add(egui::Slider::new(&mut seek_value, 0.0..=duration.max(0.1))
                                .show_value(false)
                                .trailing_fill(true)).changed() {
                                preview.seek(seek_value);
                            }
                        });

                        ui.add_space(8.0);

                        // Buttons & Time
                        ui.horizontal(|ui| {
                            let icon_color = if ui.visuals().dark_mode { [240, 240, 240, 255] } else { [60, 60, 60, 255] };

                            // Play/Pause - with hover effect
                            let play_icon = if is_playing { "pause" } else { "play" };
                            if let Some(tex) = svg_manager.get_icon(ui.ctx(), play_icon, 48, icon_color) {
                                let btn = egui::ImageButton::new(egui::load::SizedTexture::new(tex.id(), egui::vec2(22.0, 22.0)));
                                if ui.add(btn).on_hover_text(if is_playing { "Pausar" } else { "Reproduzir" }).clicked() {
                                    preview.toggle_play();
                                }
                            }

                            ui.add_space(10.0);

                            // Volume - with hover effect
                            let vol_icon = if is_muted { "vol_mute" } else { "vol_high" };
                            if let Some(tex) = svg_manager.get_icon(ui.ctx(), vol_icon, 48, icon_color) {
                                let btn = egui::ImageButton::new(egui::load::SizedTexture::new(tex.id(), egui::vec2(22.0, 22.0)));
                                if ui.add(btn).on_hover_text(if is_muted { "Ativar som" } else { "Mudo" }).clicked() {
                                    preview.toggle_mute();
                                }
                            }

                            // Volume Slider
                            let mut vol = volume;
                            ui.add_space(5.0);
                            ui.spacing_mut().slider_width = 80.0;
                            ui.visuals_mut().selection.bg_fill = crate::ui::theme::COLOR_ACCENT;
                            if ui.add(egui::Slider::new(&mut vol, 0.0..=1.0).show_value(false)).changed() {
                                preview.set_volume(vol);
                            }

                            ui.add_space(15.0);

                            // Time
                            let time_text = format!(
                                "{} / {}",
                                crate::ui::components::media_preview::format_time(current_time),
                                crate::ui::components::media_preview::format_time(duration)
                            );
                            let time_color = if ui.visuals().dark_mode { egui::Color32::LIGHT_GRAY } else { egui::Color32::DARK_GRAY };
                            ui.label(egui::RichText::new(time_text).size(13.0).color(time_color));
                            
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                // Detach Button - with hover effect
                                let detach_icon_name = if is_detached { "minimize_2" } else { "external-link" }; 
                                let tooltip = if is_detached { "Anexar ao painel" } else { "Desacoplar vídeo" };

                                if let Some(tex) = svg_manager.get_icon(ui.ctx(), detach_icon_name, 48, icon_color) {
                                    let btn = egui::ImageButton::new(egui::load::SizedTexture::new(tex.id(), egui::vec2(18.0, 18.0)));
                                    if ui.add(btn).on_hover_text(tooltip).clicked() {
                                        if is_detached && preview.is_maximized() {
                                            // Handle cleanup if re-attaching from fullscreen
                                            preview.set_fullscreen_applied(false);
                                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
                                            if preview.prev_app_maximized() {
                                                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(true));
                                            }
                                        }
                                        preview.toggle_detached();
                                    }
                                } else {
                                    if ui.button(if is_detached { "Anexar" } else { "Desacoplar" }).on_hover_text(tooltip).clicked() {
                                        if is_detached && preview.is_maximized() {
                                            preview.set_fullscreen_applied(false);
                                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
                                            if preview.prev_app_maximized() {
                                                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(true));
                                            }
                                        }
                                        preview.toggle_detached();
                                    }
                                }

                                // VSR Button (NVIDIA Video Super Resolution)
                                // Only show in detached/fullscreen mode as requested
                                if is_detached {
                                    ui.add_space(4.0);

                                    let is_vsr = preview.is_vsr_enabled();
                                    let label = if is_vsr { "VSR On" } else { "VSR Off" };
                                    
                                    // Custom style for ON state (NVIDIA Green), Standard style for OFF state
                                    let btn = if is_vsr {
                                        egui::Button::new(
                                            egui::RichText::new(label).strong().size(11.0).color(egui::Color32::WHITE)
                                        )
                                        .fill(egui::Color32::from_rgb(118, 185, 0))
                                    } else {
                                        egui::Button::new(
                                            egui::RichText::new(label).size(11.0)
                                        )
                                    };

                                    if ui.add(btn).on_hover_text(
                                        if is_vsr { "Desativar NVIDIA VSR Upscaling" } else { "Ativar NVIDIA VSR (AI Upscaling)" }
                                    ).clicked() {
                                        if let Err(e) = preview.toggle_vsr() {
                                            eprintln!("Error toggling VSR: {}", e);
                                        }
                                    }
                                }

                                // Fullscreen Button (Only in detached mode) - with hover effect
                                if is_detached {
                                    ui.add_space(4.0);
                                    let is_fullscreen = preview.is_maximized();
                                    let fs_icon_name = if is_fullscreen { "minimize" } else { "maximize" };
                                    let fs_tooltip = if is_fullscreen { "Sair da Tela Cheia (ESC)" } else { "Tela Cheia" };
                                    
                                    if let Some(tex) = svg_manager.get_icon(ui.ctx(), fs_icon_name, 48, icon_color) {
                                        let btn = egui::ImageButton::new(egui::load::SizedTexture::new(tex.id(), egui::vec2(18.0, 18.0)));
                                        if ui.add(btn).on_hover_text(fs_tooltip).clicked() {
                                            if !is_fullscreen {
                                                let was_maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
                                                preview.set_prev_app_maximized(was_maximized);
                                                preview.set_fullscreen_applied(false);
                                            } else {
                                                preview.set_fullscreen_applied(false);
                                                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
                                                if preview.prev_app_maximized() {
                                                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(true));
                                                }
                                            }
                                            preview.toggle_maximized();
                                        }
                                    } else {
                                        // Fallback text
                                        let text = if is_fullscreen { "⮌" } else { "⛶" };
                                        if ui.button(text).on_hover_text(fs_tooltip).clicked() {
                                            if !is_fullscreen {
                                                let was_maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
                                                preview.set_prev_app_maximized(was_maximized);
                                                preview.set_fullscreen_applied(false);
                                            } else {
                                                preview.set_fullscreen_applied(false);
                                                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
                                                if preview.prev_app_maximized() {
                                                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(true));
                                                }
                                            }
                                            preview.toggle_maximized();
                                        }
                                    }
                                }
                            });
                        });
                    };

                    if is_detached {
                        // === DETACHED MODE ===
                        // 1. Placeholder in Panel
                        // 1. Placeholder in Panel (Centered and Styled)
                        // Fix: Constrain height to avoid huge whitespace
                        let placeholder_height = 200.0;
                        let available_width = ui.available_width() - 32.0;
                        
                        let mut reattach_clicked = false;
                        
                        ui.allocate_ui_with_layout(
                            egui::vec2(available_width, placeholder_height),
                            egui::Layout::centered_and_justified(egui::Direction::TopDown),
                            |ui| {
                                ui.vertical_centered(|ui| {
                                    ui.label(egui::RichText::new("Vídeo Desacoplado").strong().size(16.0));
                                    ui.add_space(8.0);
                                    
                                    // Reattach Icon Button
                                    let icon_color = if ui.visuals().dark_mode { [220, 220, 220, 255] } else { [60, 60, 60, 255] };
                                    let tooltip = "Reacoplar vídeo ao painel";
    
                                    if let Some(tex) = svg_manager.get_icon(ui.ctx(), "minimize_2", 48, icon_color) {
                                        if ui.add(egui::ImageButton::new(egui::load::SizedTexture::new(tex.id(), egui::vec2(24.0, 24.0))).frame(false))
                                            .on_hover_text(tooltip)
                                            .clicked() {
                                            if preview.is_maximized() {
                                                // Handle cleanup if re-attaching from fullscreen
                                                preview.set_fullscreen_applied(false);
                                                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
                                                if preview.prev_app_maximized() {
                                                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(true));
                                                }
                                            }
                                            preview.toggle_detached();
                                            reattach_clicked = true;
                                        }
                                    } else {
                                        if ui.button("Reacoplar").on_hover_text(tooltip).clicked() {
                                            if preview.is_maximized() {
                                                preview.set_fullscreen_applied(false);
                                                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
                                                if preview.prev_app_maximized() {
                                                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(true));
                                                }
                                            }
                                            preview.toggle_detached();
                                            reattach_clicked = true;
                                        }
                                    }
                                });
                            }
                        );

                        // If reattach was clicked, return early to let the next frame handle the "Attached" state cleanly.
                        // Continuing here with mixed state (is_detached=true locally, false in struct) causes rendering glitches.
                        if reattach_clicked {
                             ui.ctx().request_repaint(); // Ensure immediate update
                             return;
                        }

                        // 2. Floating Window logic
                        let mut open = true;
                        let is_fullscreen = preview.is_maximized(); // Renamed for clarity: this is now fullscreen
                        let mut should_restore = preview.should_restore();
                        let last_known_rect = preview.get_last_window_rect();
                        
                        // Detect minimize/restore cycle to preserve window position
                        let is_minimized = ui.ctx().input(|i| i.viewport().minimized.unwrap_or(false));
                        if is_minimized && !preview.was_minimized() {
                            // Just got minimized - mark it
                            preview.set_was_minimized(true);
                        } else if !is_minimized && preview.was_minimized() {
                            // Just got restored from minimize - force window restore
                            preview.set_was_minimized(false);
                            if last_known_rect.is_some() {
                                should_restore = true;
                                preview.set_restore_needed(true);
                            }
                        }

                        if is_fullscreen {
                            // === FULLSCREEN MODE ===
                            if !preview.fullscreen_applied() {
                                if preview.prev_app_maximized() {
                                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(false));
                                }
                                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
                                preview.set_fullscreen_applied(true);
                            }

                            // Use viewport inner rect (actual drawable area)
                            let screen_rect = ui
                                .ctx()
                                .input(|i| i.viewport().inner_rect)
                                .unwrap_or_else(|| ui.ctx().screen_rect());

                            egui::Area::new(egui::Id::new("video_fullscreen"))
                                .fixed_pos(screen_rect.min)
                                .order(egui::Order::Foreground)
                                .show(ui.ctx(), |ui| {
                                    ui.set_min_size(screen_rect.size());
                                    ui.painter().rect_filled(screen_rect, 0.0, egui::Color32::BLACK);

                                    let total_size = screen_rect.size();

                                    // Autohide logic
                                    let show_controls = preview.controls_active();
                                    let control_height = if show_controls { 75.0 } else { 0.0 };
                                    let video_height = total_size.y - control_height;

                                    let video_rect = egui::Rect::from_min_size(
                                        screen_rect.min,
                                        egui::vec2(total_size.x, video_height),
                                    );

                                    // Allocate the full area
                                    let _ = ui.allocate_exact_size(total_size, egui::Sense::click());

                                    // Render Video
                                    let mut video_ui = ui.new_child(egui::UiBuilder::new().max_rect(video_rect));
                                    preview.set_forced_size(Some(video_rect.size()));
                                    preview.show(&mut video_ui, frame);

                                    // Render Controls when active
                                    if show_controls {
                                        let control_rect = egui::Rect::from_min_size(
                                            egui::pos2(screen_rect.min.x, screen_rect.min.y + video_height),
                                            egui::vec2(total_size.x, control_height),
                                        );

                                        // Background - use theme-aware colors (same as windowed mode)
                                        let bg_color = if ui.visuals().dark_mode {
                                            egui::Color32::from_rgb(35, 35, 38) // Dark mode panel background
                                        } else {
                                            egui::Color32::from_rgb(245, 245, 248) // Light mode panel background
                                        };
                                        ui.painter().rect_filled(control_rect, 0.0, bg_color);

                                        let mut control_ui = ui.new_child(egui::UiBuilder::new().max_rect(control_rect));
                                        control_ui.add_space(6.0);
                                        draw_controls(&mut control_ui, preview, control_rect.width() - 20.0, svg_manager);
                                    }

                                    // ESC to exit fullscreen
                                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                        preview.toggle_maximized();
                                        preview.set_fullscreen_applied(false);
                                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
                                        if preview.prev_app_maximized() {
                                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(true));
                                        }
                                    }

                                    // Only repaint when video is playing or controls visible (perf optimization)
                                    if preview.get_video_state().map(|s| s.is_playing).unwrap_or(false) 
                                       || preview.controls_active() {
                                        ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
                                    }
                                });
                            
                            // Handle close via ESC already above
                            
                        } else {
                            // === WINDOWED MODE ===
                            // Restore from fullscreen if needed
                            if preview.fullscreen_applied() {
                                preview.set_fullscreen_applied(false);
                                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
                                if preview.prev_app_maximized() {
                                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(true));
                                }
                            }
                            
                            // Condition Window Builder
                            let mut window_builder = egui::Window::new(&file.name)
                                .open(&mut open)
                                .collapsible(false)
                                .title_bar(true)
                                // Fix Z-Order overlap with Resize Handles (which are Foreground)
                                .order(egui::Order::Foreground);

                            if should_restore {
                                // Force restoration to previous size for one frame
                                if let Some(rect) = last_known_rect {
                                    window_builder = window_builder
                                        .current_pos(rect.min)
                                        .fixed_size(rect.size());
                                } else {
                                    let screen = ui.ctx().screen_rect();
                                    let center = screen.center();
                                    let w = 640.0;
                                    let h = 480.0;
                                    let rect = egui::Rect::from_min_size(egui::pos2(center.x - w/2.0, center.y - h/2.0), egui::vec2(w, h));
                                    window_builder = window_builder
                                        .current_pos(rect.min)
                                        .fixed_size(rect.size());
                                }
                            } else {
                                // Normal Floating State - use last known position if available
                                if let Some(rect) = last_known_rect {
                                    window_builder = window_builder
                                        .default_pos(rect.min)
                                        .default_size(rect.size())
                                        .resizable(true);
                                } else {
                                    window_builder = window_builder
                                        .default_size([640.0, 480.0])
                                        .resizable(true);
                                }
                            }
                        
                            let window_response = window_builder.show(ui.ctx(), |ui| {
                            // === TRUE AUTOHIDE IMPLEMENTATION ===
                            // Video takes 100% when idle, shrinks when controls are shown
                            
                            let total_rect = ui.available_rect_before_wrap();
                            let total_size = total_rect.size();
                            
                            // Determine if controls should be visible
                            // Primary: MPV area reports mouse activity
                            let show_controls = preview.controls_active();
                            
                            // Control bar height (only when visible)
                            let control_height = if show_controls { 75.0 } else { 0.0 };
                            
                            // Video takes remaining space
                            let video_height = total_size.y - control_height;
                            
                            let video_rect = egui::Rect::from_min_size(
                                total_rect.min,
                                egui::vec2(total_size.x, video_height)
                            );

                            // Allocate the total space (locks window size)
                            let _ = ui.allocate_exact_size(total_size, egui::Sense::hover());

                            // 1. Render Video (full height when controls hidden)
                            let mut video_ui = ui.new_child(egui::UiBuilder::new().max_rect(video_rect));
                            preview.set_forced_size(Some(video_rect.size()));
                            preview.show(&mut video_ui, frame);

                            // 2. Render Controls only when active
                            if show_controls {
                                let control_rect = egui::Rect::from_min_size(
                                    egui::pos2(total_rect.min.x, total_rect.min.y + video_height),
                                    egui::vec2(total_size.x, control_height)
                                );
                                
                                // Background - use theme-aware colors
                                let bg_color = if ui.visuals().dark_mode {
                                    egui::Color32::from_rgb(35, 35, 38) // Dark mode panel background
                                } else {
                                    egui::Color32::from_rgb(245, 245, 248) // Light mode panel background
                                };
                                ui.painter().rect_filled(control_rect, 0.0, bg_color);
                                
                                let mut control_ui = ui.new_child(egui::UiBuilder::new().max_rect(control_rect));
                                control_ui.add_space(6.0);
                                draw_controls(&mut control_ui, preview, control_rect.width() - 20.0, svg_manager);
                            }
                            
                                // Request repaint to check timeout and hide controls
                                // Only repaint when video is playing or controls visible (perf optimization)
                                if preview.get_video_state().map(|s| s.is_playing).unwrap_or(false) 
                                   || preview.controls_active() {
                                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
                                }
                        });

                            // Post-Show Logic (only for windowed mode)
                            // 1. If Normal State, update last_known_rect
                            if !should_restore {
                                if let Some(inner) = &window_response {
                                    preview.set_last_window_rect(inner.response.rect);
                                }
                            }

                            // 2. Clear Restore Flag
                            if should_restore {
                                preview.complete_restore();
                            }
                            
                            // Handle close
                            if !open {
                                if preview.is_maximized() {
                                    preview.set_fullscreen_applied(false);
                                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
                                    if preview.prev_app_maximized() {
                                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(true));
                                    }
                                }
                                preview.set_detached(false);
                            }
                        } // end windowed mode

                    } else {
                        // === ATTACHED MODE (Standard) ===
                        preview.show(ui, frame);

                        // Controls bar BELOW the video
                        ui.add_space(8.0);
                        ui.vertical(|ui| {
                            draw_controls(ui, preview, max_preview_width, svg_manager);
                        });
                    }

                } else {
                    // === THUMBNAIL WITH PLAY OVERLAY (NON-OWNER) ===
                    if let Some(tex) = &texture {
                        let image_resp = ui.add(
                            egui::Image::new(tex)
                                .max_size(max_preview_size)
                                .shrink_to_fit(),
                        );
                        let media_rect = image_resp.rect;

                        // Central play button on hover
                        let hover_pos = ui.input(|i| i.pointer.hover_pos());
                        let is_hovered = hover_pos.map_or(false, |pos| media_rect.contains(pos));

                        if is_hovered {
                    // Show play overlay for ALL video files - transcoding handles incompatible formats
                    let center_size = 64.0;
                    let center_rect = egui::Rect::from_center_size(media_rect.center(), egui::vec2(center_size, center_size));
                    ui.painter().rect_filled(center_rect, center_size / 2.0, egui::Color32::from_black_alpha(160));
                    if let Some(tex_play) = svg_manager.get_icon(ui.ctx(), "play", 96, [255, 255, 255, 255]) {
                        ui.painter().image(tex_play.id(), center_rect.shrink(14.0), egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
                    }
                }
                // Área de clique = todo o thumbnail
                if ui.interact(media_rect, egui::Id::new("video_play_overlay_1"), egui::Sense::click()).clicked() {
                    action = Some(PreviewPanelAction::RequestPlay(file.path.clone()));
                }
                    } else {
                        ui.allocate_space(egui::vec2(max_preview_width, 200.0));
                    }
                }
            } else {
                // === CURRENT FILE IS NOT A VIDEO (but media_preview exists for another file) ===
                // Show the image thumbnail, NOT the video player
                if let Some(tex) = &texture {
                    if let Some(act) = render_texture_with_overlay(ui, tex, svg_manager) {
                        action = Some(act);
                    }
                } else {
                    // Fallback for non-video items when video is present elsewhere
                    if let Some(act) = render_fallback(ui, svg_manager) {
                        action = Some(act);
                    }
                }
            }
    } else if is_video {
            // === NO ACTIVE MEDIA PREVIEW YET (Non-owner tab or first selection) ===
            // Show thumbnail with Play Overlay
            if let Some(tex) = &texture {
                let max_preview_width = ui.available_width() - 16.0;
                let max_preview_size = egui::vec2(max_preview_width, max_preview_width);

                let image_resp = ui.add(
                    egui::Image::new(tex)
                        .max_size(max_preview_size)
                        .shrink_to_fit(),
                );
                let media_rect = image_resp.rect;

                let hover_pos = ui.input(|i| i.pointer.hover_pos());
                let is_hovered = hover_pos.map_or(false, |pos| media_rect.contains(pos));

                if is_hovered {
                    // Show play overlay for ALL video files - transcoding handles incompatible formats
                    let center_size = 64.0;
                    let center_rect = egui::Rect::from_center_size(media_rect.center(), egui::vec2(center_size, center_size));
                    ui.painter().rect_filled(center_rect, center_size / 2.0, egui::Color32::from_black_alpha(160));
                    if let Some(tex_play) = svg_manager.get_icon(ui.ctx(), "play", 96, [255, 255, 255, 255]) {
                        ui.painter().image(tex_play.id(), center_rect.shrink(14.0), egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
                    }
                }
                // Área de clique = todo o thumbnail
                if ui.interact(media_rect, egui::Id::new("video_play_overlay_2"), egui::Sense::click()).clicked() {
                    action = Some(PreviewPanelAction::RequestPlay(file.path.clone()));
                }
            } else {
                ui.allocate_space(egui::vec2(ui.available_width() - 16.0, 200.0));
            }

        } else if let Some(tex) = &texture {
            // Fallback: Static Thumbnail (No MediaPreview state)
            if let Some(act) = render_texture_with_overlay(ui, tex, svg_manager) {
                action = Some(act);
            }
        } else {
           if let Some(act) = render_fallback(ui, svg_manager) {
                action = Some(act);
           }
        }
        ui.add_space(20.0);

    });

    // Tabela de Detalhes
    ui.scope(|ui| {
        ui.set_max_width(ui.available_width());
        ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
            ui.set_max_width(ui.available_width());
            // 1. Filename Header (matches Explorer style)
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.add_space(5.0);
                
                let has_button = !file.is_dir && file.drive_info.is_none();
                // Reserve space for button if needed
                let button_width = if has_button { 22.0 } else { 0.0 };
                // Calculate available width for text
                let available_width = ui.available_width() - button_width - 5.0; // -5.0 for spacing/padding

                // 1. Text Region (Centered)
                // We allocate a specific width for the text and use a top-down layout with Center alignment
                // to center the text horizontally within this region.
                ui.allocate_ui_with_layout(
                    egui::vec2(available_width, 0.0), // width=available, height=auto
                    egui::Layout::top_down(egui::Align::Center), 
                    |ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(&file.name).strong().size(15.0)
                            ).wrap()
                        );
                    }
                );

                // 2. Refresh Button (Right aligned)
                if has_button {
                    let extension = file.path.extension().and_then(|e| e.to_str()).unwrap_or("");
                    let is_media = crate::infrastructure::windows::is_media_extension(extension);
                    
                    if is_media {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                            ui.add_space(5.0); // Spacing between text and button
                            let icon_color = if ui.visuals().dark_mode { [220, 220, 220, 255] } else { [60, 60, 60, 255] };
                            if let Some(tex) = svg_manager.get_icon(ui.ctx(), "refresh", 32, icon_color) {
                                if ui.add(egui::ImageButton::new(egui::load::SizedTexture::new(tex.id(), egui::vec2(16.0, 16.0))).frame(false))
                                    .on_hover_text("Recarregar Thumbnail")
                                    .clicked() {
                                    action = Some(PreviewPanelAction::RefreshThumbnail(file.path.clone()));
                                }
                            }
                        });
                    }
                }
            });
            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);

            // HELPER: add_detail
            let add_detail = |ui: &mut egui::Ui, label: &str, value: String| {
                ui.horizontal_top(|ui| {
                    ui.add_sized(
                        egui::vec2(110.0, 0.0),
                        egui::Label::new(
                            egui::RichText::new(label).color(ui.visuals().weak_text_color()),
                        ),
                    );
                    ui.add(egui::Label::new(value).wrap());
                });
                ui.add_space(4.0);
            };

            // Remove generic "Nome" if we have the header above, or keep it if preferred.
            // Let's keep it for completeness but use the helper.
            // add_detail(ui, "Nome:", file.name.clone());

            // 2. Tipo (General)
            if file.name == "Este Computador" {
                // Caso especial: Este Computador - mostra apenas quantidade de drives
                add_detail(ui, "Tipo:", "Visão do Sistema".to_string());
                let drive_count = file.size as usize; // drive count stored in size field
                let drive_text = if drive_count == 1 {
                    "1 unidade disponível".to_string()
                } else {
                    format!("{} unidades disponíveis", drive_count)
                };
                add_detail(ui, "Unidades:", drive_text);
                // Não mostra mais detalhes para Este Computador
            } else if let Some(drive) = &file.drive_info {
                add_detail(ui, "Tipo:", format!("{:?}", drive.drive_type));
            } else if file.is_dir {
                if file.name.to_lowercase().ends_with(".zip") {
                     add_detail(ui, "Tipo:", "Arquivo ZIP".to_string());
                } else {
                     add_detail(ui, "Tipo:", "Pasta de Arquivos".to_string());
                }
            } else {
                let ext = file
                    .path
                    .extension()
                    .map(|e| e.to_string_lossy().to_string().to_uppercase())
                    .unwrap_or_else(|| "Arquivo".to_string());
                add_detail(ui, "Tipo:", format!("Arquivo {}", ext));
            }

            // 3. Metadados do Arquivo (Data/Tamanho)
            // Não aplicável para "Este Computador"
            if file.drive_info.is_none() && file.name != "Este Computador" {
                add_detail(
                    ui,
                    "Data modificada:",
                    crate::infrastructure::windows::format_date(file.modified),
                );

                // Tamanho (using helper for alignment)
                let size_str = if file.is_dir {
                    if let Some(size) = folder_size {
                        crate::infrastructure::windows::format_size(size)
                    } else {
                        "Calculando...".to_string()
                    }
                } else {
                    crate::infrastructure::windows::format_size(file.size)
                };

                add_detail(ui, "Tamanho:", size_str);

                if file.is_dir && folder_size.is_none() && !is_folder_size_loading {
                    action = Some(PreviewPanelAction::CalculateFolderSize(file.path.clone()));
                }
            }

            // 4. Metadados de Mídia (Imagens/Vídeos)
            if is_metadata_loading {
                add_detail(ui, "Metadados:", "Carregando...".to_string());
            } else if let Some(meta) = metadata {
                // Dimensões / Resolução
                if let (Some(w), Some(h)) = (meta.width, meta.height) {
                    add_detail(ui, "Resolução:", format!("{} x {} px", w, h));
                }

                // Formato / Codecs
                if let Some(fmt) = &meta.format {
                    add_detail(ui, "Formato:", fmt.clone());
                }

                if let Some(codec) = &meta.video_codec {
                    add_detail(ui, "Video Codec:", codec.clone());
                }

                if let Some(codec) = &meta.audio_codec {
                    add_detail(ui, "Audio Codec:", codec.clone());
                }

                // Audio Info
                if let Some(br) = meta.audio_bitrate {
                    add_detail(
                        ui,
                        "Audio BR:",
                        crate::infrastructure::windows::format_bitrate(br),
                    );
                }

                if let Some(channels) = meta.audio_channels {
                    let channel_name = match channels {
                        1 => "Mono",
                        2 => "Estéreo",
                        6 => "5.1",
                        8 => "7.1",
                        _ => "Outro",
                    };
                    add_detail(ui, "Canais:", format!("{} ({})", channels, channel_name));
                }

                // Video Info
                if let Some(d) = meta.duration_100ns {
                    add_detail(
                        ui,
                        "Duração:",
                        crate::infrastructure::windows::format_media_duration(d),
                    );
                }

                if let Some(fps) = meta.frame_rate {
                    add_detail(ui, "Frame rate:", format!("{:.2} fps", fps));
                }

                // Bitrate Total
                let mut bitrate_to_show = meta.bitrate;
                // If bitrate is missing OR zero, try to approximate from file size
                if bitrate_to_show.unwrap_or(0) == 0 {
                    if let Some(d) = meta.duration_100ns {
                        bitrate_to_show =
                            crate::infrastructure::windows::approximate_bitrate(file.size, d);
                    }
                }
                if let Some(bps) = bitrate_to_show.filter(|&b| b > 0) {
                    add_detail(
                        ui,
                        "Bitrate:",
                        crate::infrastructure::windows::format_bitrate(bps),
                    );
                }

                // EXIF / Camera Data
                if let Some(maker) = &meta.camera_maker {
                    add_detail(ui, "Fabricante:", maker.clone());
                }
                if let Some(model) = &meta.camera_model {
                    add_detail(ui, "Modelo:", model.clone());
                }
                if let Some(date) = &meta.date_taken {
                    add_detail(ui, "Captura:", date.clone());
                }
                if let Some(f) = &meta.f_stop {
                    add_detail(ui, "F-stop:", f.clone());
                }
                if let Some(e) = &meta.exposure_time {
                    add_detail(ui, "Exposição:", e.clone());
                }
                if let Some(iso) = meta.iso_speed {
                    add_detail(ui, "ISO:", format!("ISO-{}", iso));
                }
                if let Some(f) = &meta.focal_length {
                    add_detail(ui, "Dist. Focal:", f.clone());
                }
                if let Some(a) = &meta.max_aperture {
                    add_detail(ui, "Abertura:", a.clone());
                }
                if let Some(m) = &meta.metering_mode {
                    add_detail(ui, "Medição:", m.clone());
                }
                if let Some(f) = &meta.flash_mode {
                    add_detail(ui, "Flash:", f.clone());
                }
                if let Some(s) = &meta.subject {
                    add_detail(ui, "Assunto:", s.clone());
                }
                if let Some(depth) = meta.color_depth {
                    add_detail(ui, "Profundidade:", format!("{} bits", depth));
                }
            }

            // 5. Drive Details (Windows Explorer style)
            if let Some(drive) = &file.drive_info {
                let used_space = drive.total_space.saturating_sub(drive.free_space);

                add_detail(
                    ui,
                    "Espaço usado:",
                    crate::infrastructure::windows::format_size(used_space),
                );
                add_detail(
                    ui,
                    "Espaço livre:",
                    crate::infrastructure::windows::format_size(drive.free_space),
                );
                add_detail(
                    ui,
                    "Tamanho total:",
                    crate::infrastructure::windows::format_size(drive.total_space),
                );
                add_detail(
                    ui,
                    "Sist. Arq:",
                    if drive.file_system.is_empty() {
                        "NTFS".to_string()
                    } else {
                        drive.file_system.clone()
                    },
                );
            }

        });
    });

    action
}
