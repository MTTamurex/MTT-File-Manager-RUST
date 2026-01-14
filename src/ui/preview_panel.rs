use crate::domain::file_entry::FileEntry;
use crate::infrastructure::windows::is_media_extension;
use crate::infrastructure::windows::MediaMetadata;
use crate::ui::components::MediaPreview;
use crate::ui::icon_loader::IconLoader;
use crate::ui::svg_icons::SvgIconManager;
use crate::ui::widgets;
use eframe::egui;
use std::path::PathBuf;

pub enum PreviewPanelAction {
    RefreshThumbnail(PathBuf),
    LoadFolderPreview(PathBuf),
    CalculateFolderSize(PathBuf),
}

pub fn render_preview_panel(
    ui: &mut egui::Ui,
    file: &FileEntry,
    selected_thumbnail: Option<&egui::TextureHandle>,
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
) -> Option<PreviewPanelAction> {
    // Metadados são processados de forma assíncrona; se chegarem, o metadata será Some(...)
    let mut action = None;
    
    // Check if this is a video file
    let is_video = file.path.extension()
        .map(|ext| {
            let e = ext.to_string_lossy().to_lowercase();
            matches!(e.as_str(), "mp4" | "mkv" | "avi" | "webm" | "mov" | "wmv" | "flv")
        })
        .unwrap_or(false);

    ui.vertical_centered(|ui| {
        ui.add_space(20.0);

        // Preview de imagem/video (se houver thumbnail)
        let is_media = file
            .path
            .extension()
            .map(|ext| is_media_extension(&ext.to_string_lossy()))
            .unwrap_or(false);

        let texture = if let Some(tex) = selected_thumbnail {
            Some(tex.clone())
        } else {
            texture_cache_peek
        };

        if let Some(preview) = media_preview {
            if is_video {
                // VIDEO PLAYER LOGIC
                let is_player_visible = preview.is_player_visible();
                let video_state = preview.get_video_state();
                let is_playing = video_state.as_ref().map(|s| s.is_playing).unwrap_or(false);
                let current_time = video_state.as_ref().map(|s| s.current_time).unwrap_or(0.0);
                let duration = video_state.as_ref().map(|s| s.duration).unwrap_or(0.0);
                let volume = video_state.as_ref().map(|s| s.volume).unwrap_or(1.0);
                let is_muted = video_state.as_ref().map(|s| s.is_muted).unwrap_or(false);

                let max_preview_width = ui.available_width() - 16.0;
                let max_preview_size = egui::vec2(max_preview_width, max_preview_width);

                if is_player_visible {
                    // === ACTIVE PLAYER ===
                    preview.show(ui, frame);

                    // Controls bar BELOW the video (no extra frames/lines)
                    ui.add_space(8.0);
                    ui.vertical(|ui| {
                        ui.set_width(max_preview_width);

                        // Seek Bar - App Blue
                        ui.horizontal(|ui| {
                            ui.spacing_mut().slider_width = max_preview_width;
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

                            // Play/Pause
                            let play_icon = if is_playing { "pause" } else { "play" };
                            if let Some(tex) = svg_manager.get_icon(ui.ctx(), play_icon, 48, icon_color) {
                                if ui.add(egui::ImageButton::new(egui::load::SizedTexture::new(tex.id(), egui::vec2(22.0, 22.0))).frame(false)).clicked() {
                                    preview.toggle_play();
                                }
                            }

                            ui.add_space(10.0);

                            // Volume
                            let vol_icon = if is_muted { "vol_mute" } else { "vol_high" };
                            if let Some(tex) = svg_manager.get_icon(ui.ctx(), vol_icon, 48, icon_color) {
                                if ui.add(egui::ImageButton::new(egui::load::SizedTexture::new(tex.id(), egui::vec2(22.0, 22.0))).frame(false)).clicked() {
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
                                crate::ui::components::webview_preview::format_time(current_time),
                                crate::ui::components::webview_preview::format_time(duration)
                            );
                            let time_color = if ui.visuals().dark_mode { egui::Color32::LIGHT_GRAY } else { egui::Color32::DARK_GRAY };
                            ui.label(egui::RichText::new(time_text).size(13.0).color(time_color));
                        });
                    });
                } else {
                    // === THUMBNAIL ===
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
                            let center_size = 64.0;
                            let center_rect = egui::Rect::from_center_size(media_rect.center(), egui::vec2(center_size, center_size));
                            ui.painter().rect_filled(center_rect, center_size / 2.0, egui::Color32::from_black_alpha(160));
                            if let Some(tex) = svg_manager.get_icon(ui.ctx(), "play", 96, [255, 255, 255, 255]) {
                                ui.painter().image(tex.id(), center_rect.shrink(14.0), egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
                            }
                            if ui.put(center_rect, egui::Button::new("").frame(false)).clicked() {
                                preview.toggle_play();
                            }
                        }
                    } else {
                        ui.allocate_space(egui::vec2(max_preview_width, 200.0));
                    }
                }
            } else {
                // Show media preview for images/GIFs
                preview.show(ui, frame);

                if widgets::icon_button(ui, svg_manager, "refresh", "Recarregar Thumbnail", None)
                    .clicked()
                {
                    action = Some(PreviewPanelAction::RefreshThumbnail(file.path.clone()));
                }
            }
        } else if let (Some(tex), true) = (texture, is_media) {
            // Fallback: Show thumbnail
            let max_preview_width = ui.available_width() - 16.0;
            let max_preview_size = egui::vec2(max_preview_width, max_preview_width);

            ui.add(
                egui::Image::new(&tex)
                    .max_size(max_preview_size)
                    .shrink_to_fit(),
            );

            if widgets::icon_button(ui, svg_manager, "refresh", "Recarregar Thumbnail", None)
                .clicked()
            {
                action = Some(PreviewPanelAction::RefreshThumbnail(file.path.clone()));
            }
        } else {
            // Pasta ou Drive ou Arquivo sem Thumbnail
            let max_w: f32 = ui.available_width() - 40.0;
            let icon_size: f32 = (120.0f32).min(max_w);

            if let Some(_) = &file.drive_info {
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
            } else if file.is_dir {
                // PASTA
                if is_recycle_bin_view {
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

                    if let Some(tex) = folder_preview_peek {
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
                        action = Some(PreviewPanelAction::LoadFolderPreview(file.path.clone()));

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
                if let Some(icon) = item_icon_loader.get_or_load_icon(ui.ctx(), &file.path) {
                    ui.add(
                        egui::Image::new(&icon)
                            .max_size(egui::vec2(icon_size * 0.6, icon_size * 0.6)),
                    );
                } else {
                    ui.label(egui::RichText::new("??").size(icon_size * 0.6));
                }
            }
            ui.add_space(20.0);
        }
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
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new(&file.name).strong().size(15.0));
                });
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
            if let Some(drive) = &file.drive_info {
                add_detail(ui, "Tipo:", format!("{:?}", drive.drive_type));
            } else if file.is_dir {
                add_detail(ui, "Tipo:", "Pasta de Arquivos".to_string());
            } else {
                let ext = file
                    .path
                    .extension()
                    .map(|e| e.to_string_lossy().to_string().to_uppercase())
                    .unwrap_or_else(|| "Arquivo".to_string());
                add_detail(ui, "Tipo:", format!("Arquivo {}", ext));
            }

            // 3. Metadados do Arquivo (Data/Tamanho)
            if file.drive_info.is_none() {
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
