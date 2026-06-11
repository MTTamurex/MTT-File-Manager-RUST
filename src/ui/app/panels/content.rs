use crate::app::ImageViewerApp;
use crate::domain::file_entry::{FileEntry, SyncStatus};
use crate::domain::special_paths::{COMPUTER_VIEW_ID, RECYCLE_BIN_VIEW_ID};
use crate::infrastructure::windows as windows_infra;
use eframe::egui;
use rust_i18n::t;
use std::cell::RefCell;
use std::path::PathBuf;

// M-12: Per-frame cache for "current folder" FileEntry (when no file is selected).
// Keyed by (current_path, modified_hint); invalidated on navigation or folder update.
thread_local! {
    static FOLDER_ENTRY_CACHE: RefCell<Option<(String, u64, FileEntry)>> = RefCell::new(None);
}

pub(super) fn render_preview_panel_layout(
    app: &mut ImageViewerApp,
    ctx: &egui::Context,
    frame: &eframe::Frame,
) {
    if app.show_preview_panel {
        let defer_preview_work = app.defer_preview_work_after_selection;
        if defer_preview_work {
            app.defer_preview_work_after_selection = false;
            ctx.request_repaint();
        }

        // M-2: Only call refresh_selected_metadata when the selection actually changed.
        // The function has its own early-return for same-path, but this skips the call
        // entirely (avoiding closure + field reads) on the common no-change frame.
        let needs_metadata_refresh = match (&app.selected_file, &app.last_metadata_path) {
            (Some(f), _) if f.is_dir => false,
            (Some(f), Some(p)) => f.path != *p,
            (Some(_), None) => true, // watcher cleared last_metadata_path to force re-fetch
            (None, _) => false,
        };
        if !defer_preview_work && needs_metadata_refresh {
            app.refresh_selected_metadata();
        }

        if !defer_preview_work {
            app.warm_detail_panel_folder_preview();
        }

        // Clamp width to valid range BEFORE using it
        let target_width = app
            .layout
            .sidebar_right_width
            .clamp(super::RIGHT_SIDEBAR_MIN, super::RIGHT_SIDEBAR_MAX);

        // Use exact_width + resizable(false) to FORCE the width from app state
        // Resize is handled via manual drag handles rendered separately
        let _right_panel_response = egui::SidePanel::right("preview_panel")
            .exact_width(target_width)
            .resizable(false) // Resize handled manually via drag handles
            .frame(egui::Frame {
                fill: if ctx.style().visuals.dark_mode {
                    egui::Color32::from_rgb(45, 45, 45)
                } else {
                    egui::Color32::WHITE
                },
                inner_margin: egui::Margin {
                    left: 12,
                    right: 12,
                    top: 8,
                    bottom: 8,
                },
                ..Default::default()
            })
            .show(ctx, |ui| {
                use crate::ui::preview_panel::{render_preview_panel, PreviewPanelAction};

                egui::ScrollArea::vertical()
                    .id_salt("preview_scroll")
                    .show(ui, |ui| {
                        ui.set_max_width(ui.available_width());

                        let effective_file = calculate_effective_file(app);

                        if let Some(file) = effective_file {
                            if !defer_preview_work {
                                app.prepare_selected_preview_for_file(&file);
                            }

                            let tab_id = app.tab_manager.active().id;
                            let selected_metadata =
                                app.selected_metadata.as_ref().and_then(|(p, meta)| {
                                    if p == &file.path {
                                        Some(meta)
                                    } else {
                                        None
                                    }
                                });

                            let is_current_folder_panel =
                                is_current_folder_panel_target(app, &file);
                            let (folder_summary, is_folder_size_loading) = if file.is_dir {
                                app.folder_size_state
                                    .summary_for_panel_render(&file.path, is_current_folder_panel)
                            } else {
                                (None, false)
                            };

                            let is_owner = app.media_preview_owner_tab_id == Some(tab_id);

                            let multi_selection_total_size: u64 = app
                                .items
                                .iter()
                                .filter(|item| app.multi_selection.contains(&item.path))
                                .map(|item| item.size)
                                .sum();

// Resolution guard for preview panel: accept textures that are
                            // large enough for the detail panel, OR that are the best available
                            // when we've already attempted the required quality bucket (some
                            // video thumbnails can't be extracted at higher resolutions).
                            let preview_min_size = crate::domain::thumbnail::detail_preview_size(&file.path);
                            let required_preview_bucket = crate::workers::thumbnail::processing::get_bucket_size(
                                app.effective_thumbnail_request_size_px(preview_min_size),
                            );
                            let attempted_bucket = app.cache_manager.attempted_thumbnail_bucket_for(&file.path);
                            let texture_cache_peek = app.cache_manager.texture_cache.peek(&file.path).cloned().filter(|tex| {
                                let s = tex.size();
                                let large_enough = s[0].max(s[1]) as u32 >= preview_min_size;
                                let best_effort = attempted_bucket.is_some_and(|bucket| bucket >= required_preview_bucket);
                                large_enough || best_effort
                            });

                            let action = render_preview_panel(
                                ui,
                                &file,
                                app.multi_selection.len(),
                                multi_selection_total_size,
                                app.selected_thumbnail.as_ref(),
                                app.selected_gif.as_mut(),
                                app.media_preview.as_mut(), // Always pass mut if it exists, visibility is controlled by HWND
                                selected_metadata,
                                texture_cache_peek,
                                app.cache_manager
                                    .folder_preview_cache
                                    .get(&file.path)
                                    .cloned(),
                                app.cache_manager
                                    .folder_preview_loading
                                    .contains(&file.path),
                                app.metadata_loading.contains(&file.path),
                                folder_summary,
                                is_folder_size_loading,
                                &mut app.live_file_size_cache,
                                &mut app.live_file_size_loading,
                                &app.live_file_size_req_sender,
                                app.navigation_state.is_recycle_bin_view,
                                &mut app.item_icon_loader,
                                &mut app.svg_icon_manager,
                                Some(frame),
                                is_owner,
                                app.cache_manager.is_failed(&file.path),
                            );

                            if let Some(act) = action {
                                match act {
                                    PreviewPanelAction::RequestPlay(path) => {
                                        app.request_video_preview_playback(path);
                                    }
                                    PreviewPanelAction::DetachVideo { path, position, volume } => {
                                        // 1. Kill any existing standalone player
                                        app.kill_video_player_process();
                                        // 2. Destroy the in-process media preview (frees MPV/HWND)
                                        app.destroy_media_preview();
                                        // 3. Spawn standalone video player process
                                        if let Some(child) = crate::video_player::open_video_player(path, position, volume) {
                                            app.video_player_process = Some(child);
                                        }
                                    }
                                    PreviewPanelAction::RefreshThumbnail(path) => {
                                        // Check if it's a folder or a file
                                        let is_folder = app
                                            .selected_file
                                            .as_ref()
                                            .map(|f| f.is_dir && !f.is_archive())
                                            .unwrap_or(false);

                                        if is_folder {
                                            // Handle folder preview refresh
                                            log::debug!(
                                                "[REFRESH FOLDER PREVIEW] Starting refresh for: {:?}",
                                                path
                                            );
                                            // Clear folder preview cache
                                            app.cache_manager.folder_preview_cache.pop(&path);
                                            log::debug!("[REFRESH FOLDER PREVIEW] Folder preview cache cleared");
                                            app.cache_manager.finish_folder_preview_loading(&path);
                                            log::debug!("[REFRESH FOLDER PREVIEW] Loading state cleared");
                                            // Re-request folder preview
                                            app.request_folder_preview_load(path.clone());
                                            log::debug!(
                                                "[REFRESH FOLDER PREVIEW] Request sent for: {:?}",
                                                path
                                            );
                                            app.notifications.push(
                                                crate::application::AppNotification::info(
                                                    rust_i18n::t!("panels.refreshing_preview").to_string(),
                                                ),
                                            );
                                        } else {
                                            // Handle file thumbnail refresh (existing logic)
                                            log::debug!(
                                                "[REFRESH THUMBNAIL] Starting refresh for: {:?}",
                                                path
                                            );
                                            // Clear all caches to allow retry
                                            // PERF FIX (C-1): Dispatch SQLite cleanup to background worker
                                            // FORCED: user explicitly requested refresh
                                            app.enqueue_disk_cache_invalidations_forced(vec![path.clone()]);
                                            log::debug!("[REFRESH THUMBNAIL] Disk cache cleared");
                                            app.cache_manager.texture_cache.pop(&path);
                                            app.cache_manager.forget_attempted_thumbnail_bucket(&path);
                                            log::debug!("[REFRESH THUMBNAIL] Texture cache cleared");
                                            app.cache_manager.loading_set.remove(&path);
                                            log::debug!("[REFRESH THUMBNAIL] Loading set cleared");
                                            // CRITICAL: Also clear RAM cache (rgba_data_cache) or
                                            // request_thumbnail_load will return early without re-extracting
                                            app.cache_manager.pop_rgba_data(&path);
                                            log::debug!("[REFRESH THUMBNAIL] RGBA cache cleared");
                                            // Clear failure cache so it will be retried
                                            crate::workers::thumbnail::clear_failure_cache(&path);
                                            log::debug!("[REFRESH THUMBNAIL] Failure cache cleared");
                                            // Force regeneration by passing modified=0 (will trigger new extraction)
                                            app.request_thumbnail_load_with_modified(
                                                path.clone(),
                                                512,
                                                0,
                                            );
                                            log::debug!(
                                                "[REFRESH THUMBNAIL] Request sent to worker for: {:?}",
                                                path
                                            );
                                            app.notifications.push(
                                                crate::application::AppNotification::info(
                                                    rust_i18n::t!("panels.refreshing_thumbnail").to_string(),
                                                ),
                                            );
                                        }
                                    }
                                    PreviewPanelAction::LoadFolderPreview(path) => {
                                        app.request_folder_preview_load(path);
                                    }
                                    PreviewPanelAction::CalculateFolderSize(path) => {
                                        // Cancel any in-progress calculation before starting new one
                                        app.folder_size_state.cancel
                                            .store(true, std::sync::atomic::Ordering::Release);
                                        app.folder_size_state.loading.insert(path.clone());
                                        let _ = app.folder_size_state.req_sender.send(path);
                                    }
                                    PreviewPanelAction::VolumeChanged(vol) => {
                                        app.session_volume = vol;
                                    }
                                }
                            }
                        } else {
                            ui.vertical_centered(|ui| {
                                ui.add_space(100.0);
                                ui.label(rust_i18n::t!("panels.no_selection"));
                                ui.label(rust_i18n::t!("panels.select_hint"));
                            });
                        }
                    });
            });
    }
}

fn calculate_effective_file(app: &ImageViewerApp) -> Option<FileEntry> {
    // PERF FIX (M-1): Return a reference-based approach — clone only when the
    // selected file actually changed since last frame. For the common case
    // (same file selected across frames), we reuse the cached entry.
    if let Some(ref file) = app.selected_file {
        // PERFORMANCE FIX: NEVER call path.exists() in render loop!
        // On HDD with video playing, this causes I/O spikes every frame.
        // File existence is validated on:
        // 1. Selection (when user clicks)
        // 2. File system watcher events (auto-refresh)
        // Trust the cached state - it's updated by the file watcher.
        Some(file.clone())
    } else if app.navigation_state.is_recycle_bin_view {
        Some(FileEntry {
            path: PathBuf::from(RECYCLE_BIN_VIEW_ID),
            name: RECYCLE_BIN_VIEW_ID.to_string(),
            is_dir: true,
            size: 0,
            modified: 0,
            created: None,
            folder_cover: None,
            drive_info: None,
            sync_status: SyncStatus::None,
            is_hidden: false,
            recycle_bin: None,
        })
    } else if app.navigation_state.is_computer_view {
        // "Este Computador" - show drive count info
        Some(FileEntry {
            path: PathBuf::from(COMPUTER_VIEW_ID),
            name: COMPUTER_VIEW_ID.to_string(),
            is_dir: true,
            size: app.drive_state.disks.len() as u64, // Store drive count in size field
            modified: 0,
            created: None,
            folder_cover: None,
            drive_info: None,
            sync_status: SyncStatus::None,
            is_hidden: false,
            recycle_bin: None,
        })
    } else {
        // M-12: Check cache before rebuilding the folder entry every frame.
        let mod_hint = app
            .current_folder_modified_hint
            .as_ref()
            .and_then(|(hp, m)| {
                if hp.to_string_lossy() == app.navigation_state.current_path.as_str() && *m > 0 {
                    Some(*m)
                } else {
                    None
                }
            })
            .unwrap_or(0u64);
        let cache_hit = FOLDER_ENTRY_CACHE.with(|c| {
            c.borrow().as_ref().and_then(|(cp, cm, e)| {
                if cp == &app.navigation_state.current_path && *cm == mod_hint {
                    Some(e.clone())
                } else {
                    None
                }
            })
        });
        if let Some(entry) = cache_hit {
            return Some(entry);
        }
        let path = std::path::PathBuf::from(&app.navigation_state.current_path);
        let current_folder_modified = mod_hint; // reuse already-computed value
                                                // CRITICAL FIX: Do NOT use FileEntry::from_path() here!
                                                // from_path() calls std::fs::metadata() which uses CreateFileW internally.
                                                // On OneDrive, this can block the UI thread for 30-60s on cloud-only files.
                                                // This function runs EVERY FRAME — even a 1ms delay compounds.
                                                // Build the entry with defaults instead; size/modified are not needed for
                                                // the preview panel display of the current directory.
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        let mut entry = FileEntry {
            path: path.clone(),
            name,
            is_dir: true,
            size: 0,
            modified: current_folder_modified,
            created: None,
            folder_cover: None,
            drive_info: None,
            sync_status: SyncStatus::None,
            is_hidden: false,
            recycle_bin: None,
        };
        if path.to_string_lossy().len() <= 3 && path.to_string_lossy().contains(':') {
            // PERFORMANCE FIX: Use cached drive_info from items instead of calling
            // get_volume_info() which blocks on network drives EVERY FRAME.
            let label = app
                .drive_state
                .disks
                .iter()
                .find(|(p, _)| {
                    p.starts_with(&app.navigation_state.current_path)
                        || app.navigation_state.current_path.starts_with(p)
                })
                .map(|(_, l)| l.clone())
                .unwrap_or_else(|| app.navigation_state.current_path.clone());
            entry.name = label;

            // Try to find cached drive_info from computer view items
            let cached_info = app
                .all_items
                .iter()
                .find(|item| {
                    let item_str = item.path.to_string_lossy();
                    item_str.starts_with(&app.navigation_state.current_path)
                        || app
                            .navigation_state
                            .current_path
                            .starts_with(item_str.as_ref())
                })
                .and_then(|item| item.drive_info.clone())
                // Fallback: persistent drive_info_cache survives navigation away from computer view
                .or_else(|| {
                    app.drive_state
                        .drive_info_cache
                        .get(&app.navigation_state.current_path)
                        .cloned()
                });

            if let Some(info) = cached_info {
                entry.drive_info = Some(info);
            } else {
                // Fallback NON-BLOCKING: avoid volume probes in render loop.
                // Detailed volume info is filled asynchronously by computer view pipeline.
                let drive_type =
                    windows_infra::detect_drive_type(&app.navigation_state.current_path);
                entry.drive_info = Some(crate::domain::file_entry::DriveInfo {
                    file_system: String::new(),
                    total_space: 0,
                    free_space: 0,
                    drive_type,
                });
            }
        } else {
            entry.name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| app.navigation_state.current_path.clone());
        }
        // M-12: Update cache before returning
        FOLDER_ENTRY_CACHE.with(|c| {
            *c.borrow_mut() = Some((
                app.navigation_state.current_path.clone(),
                mod_hint,
                entry.clone(),
            ));
        });
        Some(entry)
    }
}

fn is_current_folder_panel_target(app: &ImageViewerApp, file: &FileEntry) -> bool {
    app.selected_file.is_none()
        && !app.navigation_state.is_computer_view
        && !app.navigation_state.is_recycle_bin_view
        && file.is_dir
        && file.path == PathBuf::from(&app.navigation_state.current_path)
}

pub(super) fn render_central_panel_layout(app: &mut ImageViewerApp, ctx: &egui::Context) {
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(if ctx.style().visuals.dark_mode {
            egui::Color32::from_rgb(45, 45, 45)
        } else {
            egui::Color32::WHITE
        }))
        .show(ctx, |ui| {
            // CLIP FIX: Ensure central panel content cannot overflow into sidebars.
            ui.set_clip_rect(ui.max_rect());

            if app.dual_panel_enabled {
                render_dual_panel(app, ui);
            } else {
                render_single_panel_content(app, ui);
            }
        });
}

/// Render the dual-panel split view inside the central panel area.
fn render_dual_panel(app: &mut ImageViewerApp, ui: &mut egui::Ui) {
    use crate::app::dual_panel::ActivePanel;

    let total_rect = ui.available_rect_before_wrap().intersect(ui.clip_rect());
    if total_rect.width() <= 1.0 || total_rect.height() <= 1.0 {
        return;
    }

    let separator_width = 3.0_f32.min(total_rect.width());
    let content_width = (total_rect.width() - separator_width).max(0.0);
    if content_width <= 1.0 {
        return;
    }
    let min_panel_width = 120.0_f32.min(content_width * 0.5);
    let min_split_ratio = min_panel_width / content_width;
    let max_split_ratio = 1.0 - min_split_ratio;
    let ratio = app
        .layout
        .dual_panel_split_ratio
        .clamp(min_split_ratio, max_split_ratio);
    let left_width = (content_width * ratio).floor();
    let right_width = content_width - left_width;

    let left_rect =
        egui::Rect::from_min_size(total_rect.min, egui::vec2(left_width, total_rect.height()));
    let right_rect = egui::Rect::from_min_size(
        egui::pos2(
            total_rect.min.x + left_width + separator_width,
            total_rect.min.y,
        ),
        egui::vec2(right_width, total_rect.height()),
    );

    // Draw vertical separator
    let sep_rect = egui::Rect::from_min_size(
        egui::pos2(left_rect.right(), total_rect.min.y),
        egui::vec2(separator_width, total_rect.height()),
    );
    ui.painter().rect_filled(
        sep_rect,
        0.0,
        if ui.ctx().style().visuals.dark_mode {
            egui::Color32::from_rgb(60, 60, 60)
        } else {
            egui::Color32::from_rgb(200, 200, 200)
        },
    );

    // ── Draggable separator ──
    let sep_hover_rect = sep_rect.expand(3.0);
    let sep_response = ui.allocate_rect(sep_hover_rect, egui::Sense::drag());
    if sep_response.hovered() || sep_response.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    if sep_response.dragged() {
        if let Some(pointer_pos) = ui.input(|i| i.pointer.interact_pos()) {
            let left_width = (pointer_pos.x - total_rect.left() - separator_width * 0.5)
                .clamp(min_panel_width, content_width - min_panel_width);
            app.layout.dual_panel_split_ratio =
                (left_width / content_width).clamp(min_split_ratio, max_split_ratio);
        }
    }

    // Determine which panel is active vs inactive
    let active = app.dual_panel_active;
    let (active_rect, _inactive_rect) = match active {
        ActivePanel::Left => (left_rect, right_rect),
        ActivePanel::Right => (right_rect, left_rect),
    };

    // ── Draw focus indicator (border) on the active panel ──
    let focus_color = if ui.ctx().style().visuals.dark_mode {
        egui::Color32::from_rgb(80, 160, 255) // bright blue
    } else {
        egui::Color32::from_rgb(0, 100, 220)
    };
    ui.painter().rect_stroke(
        active_rect.shrink(1.0),
        0.0,
        egui::Stroke::new(2.0, focus_color),
        egui::StrokeKind::Inside,
    );

    // ── Path header for each panel ──
    let header_height = 24.0;
    let dark = ui.ctx().style().visuals.dark_mode;

    // Left panel header
    let left_header_rect =
        egui::Rect::from_min_size(left_rect.min, egui::vec2(left_rect.width(), header_height));
    let left_header_bg = if active == ActivePanel::Left {
        if dark {
            egui::Color32::from_rgb(35, 55, 80)
        } else {
            egui::Color32::from_rgb(210, 228, 250)
        }
    } else {
        if dark {
            egui::Color32::from_rgb(50, 50, 50)
        } else {
            egui::Color32::from_rgb(240, 240, 240)
        }
    };
    ui.painter()
        .rect_filled(left_header_rect, 0.0, left_header_bg);

    // Right panel header
    let right_header_rect = egui::Rect::from_min_size(
        right_rect.min,
        egui::vec2(right_rect.width(), header_height),
    );
    let right_header_bg = if active == ActivePanel::Right {
        if dark {
            egui::Color32::from_rgb(35, 55, 80)
        } else {
            egui::Color32::from_rgb(210, 228, 250)
        }
    } else {
        if dark {
            egui::Color32::from_rgb(50, 50, 50)
        } else {
            egui::Color32::from_rgb(240, 240, 240)
        }
    };
    ui.painter()
        .rect_filled(right_header_rect, 0.0, right_header_bg);

    // Render path text in headers
    let path_display = |path: &str| -> String {
        if path == COMPUTER_VIEW_ID {
            t!("nav.computer").to_string()
        } else if path == RECYCLE_BIN_VIEW_ID {
            t!("nav.recycle_bin").to_string()
        } else {
            path.to_string()
        }
    };
    let active_path = path_display(&app.navigation_state.current_path);
    let inactive_path = app
        .dual_panel_inactive_state
        .as_ref()
        .map(|s| path_display(&s.path))
        .unwrap_or_default();

    let (left_path, right_path) = match active {
        ActivePanel::Left => (active_path, inactive_path),
        ActivePanel::Right => (inactive_path, active_path),
    };

    let header_text_color = if dark {
        egui::Color32::from_rgb(220, 220, 220)
    } else {
        egui::Color32::from_rgb(30, 30, 30)
    };

    let render_header_contents = |ui: &mut egui::Ui,
                                  header_rect: egui::Rect,
                                  path: &str,
                                  header_text_color: egui::Color32|
     -> bool {
        let inner_rect = header_rect.shrink2(egui::vec2(6.0, 2.0));
        let close_size = egui::vec2(18.0, 18.0);
        let close_rect = egui::Rect::from_center_size(
            egui::pos2(
                inner_rect.right() - close_size.x * 0.5,
                inner_rect.center().y,
            ),
            close_size,
        );
        let label_rect = egui::Rect::from_min_max(
            inner_rect.min,
            egui::pos2(
                (close_rect.left() - 4.0).max(inner_rect.left()),
                inner_rect.max.y,
            ),
        );

        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(label_rect), |ui| {
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(path)
                            .size(11.0)
                            .color(header_text_color),
                    )
                    .truncate(),
                );
            });
        });

        let mut clicked = false;
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(close_rect), |ui| {
            let btn =
                egui::Button::new(egui::RichText::new("X").size(12.0).color(header_text_color))
                    .min_size(close_size)
                    .frame(false);
            clicked = ui
                .add(btn)
                .on_hover_text(t!("panels.close_panel"))
                .clicked();
        });

        clicked
    };

    // ── Close button tracking ──
    let mut close_panel = None;

    // Left path label + close button
    if render_header_contents(ui, left_header_rect, &left_path, header_text_color) {
        close_panel = Some(ActivePanel::Left);
    }

    // Right path label + close button
    if render_header_contents(ui, right_header_rect, &right_path, header_text_color) {
        close_panel = Some(ActivePanel::Right);
    }
    let close_clicked = close_panel.is_some();

    // ── Content areas (below header) ──
    let left_content_rect = egui::Rect::from_min_max(
        egui::pos2(left_rect.min.x, left_rect.min.y + header_height),
        left_rect.max,
    );
    let right_content_rect = egui::Rect::from_min_max(
        egui::pos2(right_rect.min.x, right_rect.min.y + header_height),
        right_rect.max,
    );

    let (active_content_rect, inactive_content_rect) = match active {
        ActivePanel::Left => (left_content_rect, right_content_rect),
        ActivePanel::Right => (right_content_rect, left_content_rect),
    };

    // ── Cross-panel drag target: pre-set before rendering so the active
    //    panel's bridge code doesn't cancel the drag when mouse is over
    //    the inactive panel. ──
    let file_panel_input_blocked = app.file_panel_input_blocked_by_drag_move_confirmation();
    if app.is_item_dragging && !file_panel_input_blocked {
        let hover_pos = ui.input(|i| i.pointer.hover_pos());
        if let Some(pos) = hover_pos {
            let inactive_header = match active {
                ActivePanel::Left => right_header_rect,
                ActivePanel::Right => left_header_rect,
            };
            if inactive_content_rect.contains(pos) || inactive_header.contains(pos) {
                // Mouse is over the inactive panel — set cross-panel drop target
                app.drag_cross_panel_target = app
                    .dual_panel_inactive_state
                    .as_ref()
                    .map(|s| std::path::PathBuf::from(&s.path));
            } else {
                app.drag_cross_panel_target = None;
            }
        } else {
            app.drag_cross_panel_target = None;
        }
    } else {
        app.drag_cross_panel_target = None;
    }

    // ── Render ACTIVE panel content with unique ID scope ──
    let active_id = match active {
        ActivePanel::Left => "dual_left",
        ActivePanel::Right => "dual_right",
    };
    let central_clip = ui.clip_rect().intersect(total_rect);
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(active_content_rect), |ui| {
        ui.push_id(active_id, |ui| {
            ui.set_clip_rect(active_content_rect.intersect(central_clip));
            render_single_panel_content(app, ui);
        });
    });

    // ── Render INACTIVE panel content with unique ID scope ──
    let inactive_id = match active {
        ActivePanel::Left => "dual_right",
        ActivePanel::Right => "dual_left",
    };
    app.with_inactive_panel(|app_with_inactive| {
        // The unfocused pane is still visible; only route its thumbnail requests
        // through the active generation so the shared workers accept them.
        app_with_inactive.use_active_generation_for_thumbnail_requests = true;
        app_with_inactive.suppress_file_panel_keyboard = true;
        app_with_inactive.drag_drop_cross_panel_context = true;
        ui.allocate_new_ui(
            egui::UiBuilder::new().max_rect(inactive_content_rect),
            |ui| {
                ui.push_id(inactive_id, |ui| {
                    ui.set_clip_rect(inactive_content_rect.intersect(central_clip));
                    render_single_panel_content(app_with_inactive, ui);
                });
            },
        );
        app_with_inactive.drag_drop_cross_panel_context = false;
        app_with_inactive.use_active_generation_for_thumbnail_requests = false;
        app_with_inactive.suppress_file_panel_keyboard = false;
    });

    // ── Click-to-focus: detect click AFTER rendering so item interactions
    //    are processed first. Only primary-button clicks may switch focus;
    //    right/middle clicks are reserved for context menus and other UI. ──
    // Guard: do NOT process panel clicks while the global search overlay is open.
    // The overlay uses an egui::Area as a backdrop which consumes the click, but
    // ui.input(pointer.primary_clicked()) reads raw events and ignores consumption,
    // so without this guard clicks on the overlay would switch panel focus.
    let (pointer_pos, primary_clicked) =
        ui.input(|i| (i.pointer.hover_pos(), i.pointer.primary_clicked()));
    if primary_clicked && !app.global_search.active && !file_panel_input_blocked && !close_clicked {
        if let Some(pos) = pointer_pos {
            // Switch focus when clicking in the inactive panel area.
            let inactive_header = match active {
                ActivePanel::Left => right_header_rect,
                ActivePanel::Right => left_header_rect,
            };
            if inactive_content_rect.contains(pos) || inactive_header.contains(pos) {
                app.dual_panel_switch_active();
            }
        }
    }

    // Close the clicked physical panel and let the remaining panel occupy the view.
    if let Some(panel_to_close) = close_panel {
        if panel_to_close == app.dual_panel_active {
            app.dual_panel_switch_active();
        }
        app.dual_panel_disable();
    }
}

/// Render a single panel's content (loading / empty / grid or list view).
/// Extracted so it can be used by both single and dual panel modes.
fn render_single_panel_content(app: &mut ImageViewerApp, ui: &mut egui::Ui) {
    use crate::domain::file_entry::ViewMode;

    let file_panel_input_blocked = app.file_panel_input_blocked_by_drag_move_confirmation();

    if app.is_loading_folder && app.items.is_empty() {
        ui.centered_and_justified(|ui| {
            ui.label(rust_i18n::t!("panels.loading"));
        });

        // During loading, still update drag target so cursor feedback
        // isn't stale from the previous tab's hovered folder.
        if app.is_item_dragging && !file_panel_input_blocked {
            app.update_item_drag_target_from_hover(None);
            let (ctrl, shift, primary_released) = ui.input(|i| {
                (
                    i.modifiers.ctrl,
                    i.modifiers.shift,
                    i.pointer.primary_released(),
                )
            });
            // When the mouse is over the inactive panel (cross-panel drag),
            // defer to the inactive panel's handler so drag_target_folder
            // is resolved from the inactive panel's items (subfolder support).
            if primary_released
                && (app.drag_cross_panel_target.is_none() || app.drag_drop_cross_panel_context)
            {
                app.complete_item_drag(ctrl, shift);
            }
        }
    } else if app.items.is_empty() {
        let response = ui
            .centered_and_justified(|ui| {
                ui.label(rust_i18n::t!("panels.empty_folder"));
            })
            .response
            .on_hover_cursor(egui::CursorIcon::Default);

        // During an active drag, update the drop target to the current folder
        // even though there are no items to hover over.
        if app.is_item_dragging && !file_panel_input_blocked {
            app.update_item_drag_target_from_hover(None);
            let (ctrl, shift, primary_released) = ui.input(|i| {
                (
                    i.modifiers.ctrl,
                    i.modifiers.shift,
                    i.pointer.primary_released(),
                )
            });
            // When the mouse is over the inactive panel (cross-panel drag),
            // defer to the inactive panel's handler so drag_target_folder
            // is resolved from the inactive panel's items (subfolder support).
            if primary_released
                && (app.drag_cross_panel_target.is_none() || app.drag_drop_cross_panel_context)
            {
                app.complete_item_drag(ctrl, shift);
            }
        }

        // Handle context menu on empty area
        let interact_response = ui
            .interact(
                response.rect,
                ui.id().with("empty_bg"),
                egui::Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::Default);

        if !file_panel_input_blocked
            && interact_response.secondary_clicked()
            && app.can_open_empty_area_context_menu()
        {
            app.context_menu.target_paths.clear();

            // Use current path for shell menu
            let paths = if app.navigation_state.is_recycle_bin_view {
                vec![]
            } else {
                vec![std::path::PathBuf::from(&app.navigation_state.current_path)]
            };

            // Prepare state
            let pos = ui.input(|i| i.pointer.hover_pos().unwrap_or_default());
            let right_bound = ui.available_rect_before_wrap().right();

            // Set state first
            app.context_menu
                .open(pos, right_bound, None, paths.clone(), true);

            // Then populate items
            app.populate_context_menu(ui.ctx(), &paths, true, None);
        }
    } else {
        let t_view_render = std::time::Instant::now();
        match app.view_mode {
            ViewMode::Grid => app.render_grid_view(ui),
            ViewMode::List => app.render_list_view(ui),
        }
        let view_ms = t_view_render.elapsed().as_millis();
        if view_ms > 120 {
            log::warn!(
                "[PERF-CENTRAL] Slow list/grid render: {}ms view={:?} items={} all_items={} search_len={} loading={} pending_thumbs={} pending_uploads={} visible_range={:?}",
                view_ms,
                app.view_mode,
                app.items.len(),
                app.all_items.len(),
                app.search_query.len(),
                app.is_loading_folder,
                app.pending_thumbnails.len(),
                app.cache_manager.pending_upload_set.len(),
                app.visible_index_range,
            );
        }

        if app.is_loading_folder {
            let rect = ui.max_rect();
            let status_rect = egui::Rect::from_min_size(
                rect.right_bottom() - egui::vec2(124.0, 22.0),
                egui::vec2(110.0, 16.0),
            );
            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(status_rect), |ui| {
                ui.label(
                    egui::RichText::new(rust_i18n::t!("panels.updating").to_string())
                        .size(11.0)
                        .color(egui::Color32::from_gray(130)),
                );
            });
        }
    }
}
