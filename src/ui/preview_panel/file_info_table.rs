use crate::domain::file_entry::FileEntry;
use crate::infrastructure::windows::MediaMetadata;
use crate::ui::preview_panel::actions::PreviewPanelAction;
use crate::ui::svg_icons::SvgIconManager;
use eframe::egui;

pub fn render_file_info_table(
    ui: &mut egui::Ui,
    file: &FileEntry,
    metadata: Option<&MediaMetadata>,
    folder_size: Option<u64>,
    is_folder_size_loading: bool,
    is_metadata_loading: bool,
    svg_manager: &mut SvgIconManager,
) -> Option<PreviewPanelAction> {
    let mut action = None;

    ui.scope(|ui| {
        ui.set_max_width(ui.available_width());
        ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
            ui.set_max_width(ui.available_width());
            // 1. Filename Header (matches Explorer style)
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.add_space(5.0);

                // Show button for: files (non-folders) that are not drives, OR for folders (to refresh preview)
                let has_button = (!file.is_dir && file.drive_info.is_none())
                    || (file.is_dir && !file.is_archive());
                // Reserve space for button if needed
                let button_width = if has_button { 22.0 } else { 0.0 };
                // Calculate available width for text
                let available_width = ui.available_width() - button_width - 5.0; // -5.0 for spacing/padding

                // 1. Text Region (Centered)
                ui.allocate_ui_with_layout(
                    egui::vec2(available_width, 0.0), // width=available, height=auto
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        ui.add(
                            egui::Label::new(egui::RichText::new(&file.name).strong().size(15.0))
                                .wrap(),
                        );
                    },
                );

                // 2. Refresh Button (Right aligned)
                if has_button {
                    // Show refresh button for media files OR folders (in grid view with preview)
                    let is_media = file.is_media();
                    let is_folder = file.is_dir && !file.is_archive();

                    if is_media || is_folder {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                            ui.add_space(5.0); // Spacing between text and button
                            let icon_color = if ui.visuals().dark_mode {
                                [220, 220, 220, 255]
                            } else {
                                [60, 60, 60, 255]
                            };
                            if let Some(tex) =
                                svg_manager.get_icon(ui.ctx(), "refresh", 32, icon_color)
                            {
                                let hover_text = if is_folder {
                                    "Recarregar Preview da Pasta"
                                } else {
                                    "Recarregar Thumbnail"
                                };
                                if ui
                                    .add(
                                        egui::ImageButton::new(egui::load::SizedTexture::new(
                                            tex.id(),
                                            egui::vec2(16.0, 16.0),
                                        ))
                                        .frame(false),
                                    )
                                    .on_hover_text(hover_text)
                                    .clicked()
                                {
                                    action = Some(PreviewPanelAction::RefreshThumbnail(
                                        file.path.clone(),
                                    ));
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

            // 2. Type (General)
            if file.name == "Este Computador" {
                add_detail(ui, "Tipo:", "Visão do Sistema".to_string());
                let drive_count = file.size as usize; // drive count stored in size field
                let drive_text = if drive_count == 1 {
                    "1 unidade disponível".to_string()
                } else {
                    format!("{} unidades disponíveis", drive_count)
                };
                add_detail(ui, "Unidades:", drive_text);
            } else if let Some(drive) = &file.drive_info {
                add_detail(ui, "Tipo:", format!("{:?}", drive.drive_type));
            } else if file.is_dir {
                if let Some(label) = crate::domain::file_entry::archive_type_label(&file.name) {
                    add_detail(ui, "Tipo:", label.to_string());
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

            // 3. File Metadata (Date/Size)
            if file.drive_info.is_none() && file.name != "Este Computador" {
                let is_recycle_item =
                    file.recycle_original_path.is_some() || file.deletion_date.is_some();
                let (date_label, date_value) = if is_recycle_item {
                    let value = if file.modified > 0 {
                        crate::infrastructure::windows::format_date(file.modified)
                    } else {
                        file.deletion_date
                            .clone()
                            .unwrap_or_else(|| "-".to_string())
                    };
                    ("Data de exclusão:", value)
                } else {
                    (
                        "Data modificada:",
                        crate::infrastructure::windows::format_date(file.modified),
                    )
                };
                add_detail(ui, date_label, date_value);

                let size_str = if file.is_dir && !file.is_archive() {
                    if let Some(size) = folder_size {
                        let formatted = crate::infrastructure::windows::format_size(size);
                        if is_folder_size_loading {
                            format!("{formatted} (calculando...)")
                        } else {
                            formatted
                        }
                    } else {
                        "Calculando...".to_string()
                    }
                } else {
                    crate::infrastructure::windows::format_size(file.size)
                };

                add_detail(ui, "Tamanho:", size_str);

                if file.is_dir
                    && !file.is_archive()
                    && folder_size.is_none()
                    && !is_folder_size_loading
                {
                    action = Some(PreviewPanelAction::CalculateFolderSize(file.path.clone()));
                }
            }

            // 4. Media Metadata (Images/Videos)
            if is_metadata_loading {
                add_detail(ui, "Metadados:", "Carregando...".to_string());
            } else if let Some(meta) = metadata {
                if let (Some(w), Some(h)) = (meta.width, meta.height) {
                    add_detail(ui, "Resolução:", format!("{} x {} px", w, h));
                }

                if let Some(fmt) = &meta.format {
                    add_detail(ui, "Formato:", fmt.clone());
                }

                if let Some(codec) = &meta.video_codec {
                    add_detail(ui, "Video Codec:", codec.clone());
                }

                if let Some(codec) = &meta.audio_codec {
                    add_detail(ui, "Audio Codec:", codec.clone());
                }

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

                let mut bitrate_to_show = meta.bitrate;
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

            // 5. Drive Details
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
