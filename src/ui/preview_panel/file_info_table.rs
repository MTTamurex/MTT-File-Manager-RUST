use crate::domain::file_entry::FileEntry;
use crate::domain::special_paths::COMPUTER_VIEW_ID;
use crate::infrastructure::windows::MediaMetadata;
use crate::ui::cache::FxHashSet;
use crate::ui::preview_panel::actions::PreviewPanelAction;
use crate::ui::svg_icons::SvgIconManager;
use eframe::egui;
use lru::LruCache;
use rust_i18n::t;

#[derive(Clone, Copy)]
struct LiveFileStat {
    checked_at: f64,
    size: u64,
}

fn resolve_live_file_size(
    ui: &egui::Ui,
    file: &FileEntry,
    is_metadata_loading: bool,
    live_file_size_cache: &mut LruCache<std::path::PathBuf, (u64, u64)>,
    live_file_size_loading: &mut FxHashSet<std::path::PathBuf>,
    live_file_size_req_sender: &std::sync::mpsc::Sender<crate::app::live_file_size::LiveFileSizeRequest>,
) -> u64 {
    if file.is_dir {
        return file.size;
    }

    if is_metadata_loading && file.is_media() {
        return file.size;
    }

    let now = ui.input(|i| i.time);
    let cache_id = egui::Id::new("preview_live_file_size").with(&file.path);
    let mut resolved = file.size;

    ui.ctx().data_mut(|d| {
        let mut state = d.get_temp::<LiveFileStat>(cache_id).unwrap_or(LiveFileStat {
            checked_at: -10.0,
            size: file.size,
        });

        if (now - state.checked_at) >= 1.0 {
            state.size = crate::app::live_file_size::resolve_cached_or_enqueue_live_file_size(
                &file.path,
                file.modified,
                file.size,
                live_file_size_cache,
                live_file_size_loading,
                live_file_size_req_sender,
            );
            state.checked_at = now;
            d.insert_temp(cache_id, state);
        }

        resolved = state.size;
    });

    resolved
}

pub fn render_file_info_table(
    ui: &mut egui::Ui,
    file: &FileEntry,
    metadata: Option<&MediaMetadata>,
    folder_size: Option<u64>,
    is_folder_size_loading: bool,
    is_metadata_loading: bool,
    live_file_size_cache: &mut LruCache<std::path::PathBuf, (u64, u64)>,
    live_file_size_loading: &mut FxHashSet<std::path::PathBuf>,
    live_file_size_req_sender: &std::sync::mpsc::Sender<crate::app::live_file_size::LiveFileSizeRequest>,
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

                // Show refresh button only for image/video files that generate thumbnails.
                // Folders and drives do not get this button.
                let has_button =
                    !file.is_dir && file.drive_info.is_none() && file.is_media();
                // Reserve space for button if needed
                let button_width = if has_button { 22.0 } else { 0.0 };
                // Calculate available width for text
                let available_width = ui.available_width() - button_width - 5.0; // -5.0 for spacing/padding

                // 1. Text Region (Centered)
                ui.allocate_ui_with_layout(
                    egui::vec2(available_width, 0.0), // width=available, height=auto
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        let display_name = if file.name == COMPUTER_VIEW_ID {
                            t!("nav.computer").to_string()
                        } else if file.name == crate::domain::special_paths::RECYCLE_BIN_VIEW_ID {
                            t!("nav.recycle_bin").to_string()
                        } else {
                            crate::ui::components::item_slot::display_name_for_item(file).to_string()
                        };
                        ui.add(
                            egui::Label::new(egui::RichText::new(&display_name).strong().size(15.0))
                                .wrap(),
                        );
                    },
                );

                // 2. Refresh Button (Right aligned) — only for media files
                if has_button {
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
                            if ui
                                .add(
                                    egui::ImageButton::new(egui::load::SizedTexture::new(
                                        tex.id(),
                                        egui::vec2(16.0, 16.0),
                                    ))
                                    .frame(false),
                                )
                                .on_hover_text(t!("status_bar.reload_thumbnail"))
                                .clicked()
                            {
                                action = Some(PreviewPanelAction::RefreshThumbnail(
                                    file.path.clone(),
                                ));
                            }
                        }
                    });
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
            if file.name == COMPUTER_VIEW_ID {
                add_detail(ui, &t!("file_info.type"), t!("file_info.type_system_view").to_string());
                let drive_count = file.size as usize;
                let drive_text = if drive_count == 1 {
                    t!("file_info.drives_one").to_string()
                } else {
                    t!("file_info.drives_many", count = drive_count).to_string()
                };
                add_detail(ui, &t!("file_info.drives_label"), drive_text);
            } else if let Some(drive) = &file.drive_info {
                add_detail(ui, &t!("file_info.type"), format!("{:?}", drive.drive_type));
            } else if file.is_dir {
                if let Some(label) = crate::domain::file_entry::archive_type_label(&file.name) {
                    add_detail(ui, &t!("file_info.type"), label);
                } else {
                    add_detail(ui, &t!("file_info.type"), t!("file_info.folder").to_string());
                }
            } else {
                let ext = file
                    .path
                    .extension()
                    .map(|e| e.to_string_lossy().to_string().to_uppercase())
                    .unwrap_or_else(|| t!("file_info.file_unknown").to_string());
                add_detail(ui, &t!("file_info.type"), t!("file_info.file_generic", ext = ext).to_string());
            }

            // 3. File Metadata (Date/Size)
            if file.drive_info.is_none() && file.name != COMPUTER_VIEW_ID {
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
                    (t!("file_info.date_deleted").to_string(), value)
                } else {
                    (
                        t!("file_info.date_modified").to_string(),
                        crate::infrastructure::windows::format_date(file.modified),
                    )
                };
                add_detail(ui, &date_label, date_value);

                let size_str = if file.is_dir && !file.is_archive() {
                    if let Some(size) = folder_size {
                        let formatted = crate::infrastructure::windows::format_size(size);
                        if is_folder_size_loading {
                            format!("{formatted} ({})", t!("file_info.calculating"))
                        } else {
                            formatted
                        }
                    } else {
                        t!("file_info.calculating").to_string()
                    }
                } else {
                    let live_size = resolve_live_file_size(
                        ui,
                        file,
                        is_metadata_loading,
                        live_file_size_cache,
                        live_file_size_loading,
                        live_file_size_req_sender,
                    );
                    crate::infrastructure::windows::format_size(live_size)
                };

                add_detail(ui, &t!("file_info.size"), size_str);

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
                add_detail(ui, &t!("file_info.metadata"), t!("file_info.metadata_loading").to_string());
            } else if let Some(meta) = metadata {
                if let (Some(w), Some(h)) = (meta.width, meta.height) {
                    add_detail(ui, &t!("file_info.resolution"), format!("{} x {} px", w, h));
                }

                if let Some(fmt) = &meta.format {
                    add_detail(ui, &t!("file_info.format"), fmt.clone());
                }

                if let Some(codec) = &meta.video_codec {
                    add_detail(ui, &t!("file_info.video_codec"), codec.clone());
                }

                if let Some(codec) = &meta.audio_codec {
                    add_detail(ui, &t!("file_info.audio_codec"), codec.clone());
                }

                if let Some(br) = meta.audio_bitrate {
                    add_detail(
                        ui,
                        &t!("file_info.audio_bitrate"),
                        crate::infrastructure::windows::format_bitrate(br),
                    );
                }

                if let Some(channels) = meta.audio_channels {
                    let channel_name = match channels {
                        1 => t!("file_info.channel_mono").to_string(),
                        2 => t!("file_info.channel_stereo").to_string(),
                        6 => "5.1".to_string(),
                        8 => "7.1".to_string(),
                        _ => t!("file_info.channel_other").to_string(),
                    };
                    add_detail(ui, &t!("file_info.channels"), format!("{} ({})", channels, channel_name));
                }

                if let Some(d) = meta.duration_100ns {
                    add_detail(
                        ui,
                        &t!("file_info.duration"),
                        crate::infrastructure::windows::format_media_duration(d),
                    );
                }

                if let Some(fps) = meta.frame_rate {
                    add_detail(ui, &t!("file_info.frame_rate"), format!("{:.2} fps", fps));
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
                        &t!("file_info.bitrate"),
                        crate::infrastructure::windows::format_bitrate(bps),
                    );
                }

                if let Some(maker) = &meta.camera_maker {
                    add_detail(ui, &t!("file_info.camera_maker"), maker.clone());
                }
                if let Some(model) = &meta.camera_model {
                    add_detail(ui, &t!("file_info.camera_model"), model.clone());
                }
                if let Some(date) = &meta.date_taken {
                    add_detail(ui, &t!("file_info.capture_date"), date.clone());
                }
                if let Some(f) = &meta.f_stop {
                    add_detail(ui, &t!("file_info.fstop"), f.clone());
                }
                if let Some(e) = &meta.exposure_time {
                    add_detail(ui, &t!("file_info.exposure"), e.clone());
                }
                if let Some(iso) = meta.iso_speed {
                    add_detail(ui, &t!("file_info.iso"), format!("ISO-{}", iso));
                }
                if let Some(f) = &meta.focal_length {
                    add_detail(ui, &t!("file_info.focal_length"), f.clone());
                }
                if let Some(a) = &meta.max_aperture {
                    add_detail(ui, &t!("file_info.aperture"), a.clone());
                }
                if let Some(m) = &meta.metering_mode {
                    add_detail(ui, &t!("file_info.metering"), m.clone());
                }
                if let Some(f) = &meta.flash_mode {
                    add_detail(ui, &t!("file_info.flash"), f.clone());
                }
                if let Some(s) = &meta.subject {
                    add_detail(ui, &t!("file_info.subject"), s.clone());
                }
                if let Some(depth) = meta.color_depth {
                    add_detail(ui, &t!("file_info.bit_depth"), format!("{} bits", depth));
                }
                if let Some(sr) = meta.audio_sample_rate {
                    add_detail(ui, &t!("file_info.sample_rate"), format!("{} Hz", sr));
                }
                if let Some(title) = &meta.track_title {
                    add_detail(ui, &t!("file_info.track_title"), title.clone());
                }
                if let Some(artist) = &meta.artist {
                    add_detail(ui, &t!("file_info.artist"), artist.clone());
                }
                if let Some(album) = &meta.album {
                    add_detail(ui, &t!("file_info.album"), album.clone());
                }
                if let Some(genre) = &meta.genre {
                    add_detail(ui, &t!("file_info.genre"), genre.clone());
                }
                if let Some(year) = meta.year {
                    add_detail(ui, &t!("file_info.year"), year.to_string());
                }
            }

            // 5. Drive Details
            if let Some(drive) = &file.drive_info {
                let used_space = drive.total_space.saturating_sub(drive.free_space);

                add_detail(
                    ui,
                    &t!("file_info.used_space"),
                    crate::infrastructure::windows::format_size(used_space),
                );
                add_detail(
                    ui,
                    &t!("file_info.free_space"),
                    crate::infrastructure::windows::format_size(drive.free_space),
                );
                add_detail(
                    ui,
                    &t!("file_info.total_space"),
                    crate::infrastructure::windows::format_size(drive.total_space),
                );
                add_detail(
                    ui,
                    &t!("file_info.filesystem"),
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
