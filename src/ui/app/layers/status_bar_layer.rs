use crate::app::ImageViewerApp;
use eframe::egui;

pub(crate) fn render_status_bar_layer(app: &mut ImageViewerApp, ctx: &egui::Context) {
    // Detect bulk scan completion: scan thread finished AND queue fully drained
    let is_scanning = app.bulk_thumbnail_scanning.load(std::sync::atomic::Ordering::Relaxed);
    let bulk_total = app.bulk_thumbnail_total.load(std::sync::atomic::Ordering::Relaxed);
    let queue_pending = app.thumbnail_queue.pending_count();

    // Completion = was scanning, scan thread finished, and queue is empty
    if app.bulk_thumbnail_was_scanning && !is_scanning && bulk_total > 0 && queue_pending == 0 {
        app.notifications.push(
            crate::application::AppNotification::success(
                format!("Bulk thumbnail extraction complete! ({} files)", bulk_total)
            ).with_duration(std::time::Duration::from_secs(6))
        );
        app.bulk_thumbnail_total.store(0, std::sync::atomic::Ordering::Relaxed);
        app.bulk_thumbnail_was_scanning = false;
    } else if !is_scanning && !app.bulk_thumbnail_was_scanning {
        // Not scanning — nothing to do
    } else {
        app.bulk_thumbnail_was_scanning = is_scanning || queue_pending > 0;
    }

    // Calculate progress for status bar display
    let bulk_progress = if bulk_total > 0 && (is_scanning || queue_pending > 0) {
        let done = bulk_total.saturating_sub(queue_pending);
        ctx.request_repaint(); // Keep UI refreshing while processing
        Some((done, bulk_total))
    } else {
        None
    };

    egui::TopBottomPanel::bottom("status_bar")
        .exact_height(24.0)
        .show(ctx, |ui| {
            use crate::ui::status_bar::{render_status_bar, StatusBarAction};
            let action = render_status_bar(
                ui,
                &mut app.svg_icon_manager,
                &mut app.is_loading_folder,
                app.total_items,
                &mut app.view_mode,
                &mut app.sort_mode,
                &mut app.sort_descending,
                &mut app.folders_position,
                &app.cache_manager.texture_cache,
                app.frame_time_avg_ms,
                app.frame_time_peak_ms,
                app.fps_avg,
                app.upload_budget_ms,
                app.navigation_state.is_computer_view,
                bulk_progress,
                app.current_folder_locked,
                &mut app.show_hidden_files,
            );
            match action {
                StatusBarAction::SortChanged => {
                    if app.navigation_state.is_computer_view {
                        app.sort_mode_computer = app.sort_mode;
                    } else {
                        app.sort_mode_normal = app.sort_mode;
                    }
                    if !app.current_folder_locked {
                        app.sort_descending_normal = app.sort_descending;
                        app.folders_position_normal = app.folders_position;
                    }
                    app.sort_items();
                    app.save_preferences();
                }
                StatusBarAction::OpenVirtualDriveSettings => {
                    app.navigation_state.show_virtual_drive_settings = true;
                }
                StatusBarAction::BulkThumbnailScan => {
                    let root = std::path::PathBuf::from(&app.navigation_state.current_path);
                    let queue = app.thumbnail_queue.clone();
                    let generation = app.generation;
                    let scanning_flag = app.bulk_thumbnail_scanning.clone();
                    let total_flag = app.bulk_thumbnail_total.clone();
                    let ctx_clone = app.ui_ctx.clone();
                    let disk_cache = app.disk_cache.clone();

                    scanning_flag.store(true, std::sync::atomic::Ordering::Relaxed);
                    total_flag.store(0, std::sync::atomic::Ordering::Relaxed);
                    app.notifications.push(
                        crate::application::AppNotification::info(
                            "Bulk thumbnail scan started..."
                        )
                    );

                    std::thread::Builder::new()
                        .name("bulk-thumbnail-scan".into())
                        .spawn(move || {
                            use crate::infrastructure::windows::is_media_extension;
                            use crate::workers::thumbnail::ThumbnailPriority;


                            for entry in walkdir::WalkDir::new(&root)
                                .follow_links(false)
                                .into_iter()
                                .filter_map(|e| e.ok())
                            {
                                if !scanning_flag.load(std::sync::atomic::Ordering::Relaxed) {
                                    break; // Cancelled
                                }
                                if !entry.file_type().is_file() {
                                    continue;
                                }
                                let path = entry.path();
                                let ext = match path.extension().and_then(|e| e.to_str()) {
                                    Some(e) => e.to_lowercase(),
                                    None => continue,
                                };
                                if !is_media_extension(&ext) {
                                    continue;
                                }
                                // Skip if already cached in disk_cache
                                let modified = entry.metadata().ok()
                                    .and_then(|m| m.modified().ok())
                                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                                if disk_cache.get(path, modified).is_some() {
                                    continue;
                                }
                                queue.push(
                                    path.to_path_buf(),
                                    generation,
                                    512,
                                    ThumbnailPriority::Prefetch,
                                    modified
                                        .duration_since(std::time::SystemTime::UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_secs(),
                                );
                                total_flag.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            }
                            scanning_flag.store(false, std::sync::atomic::Ordering::Relaxed);
                            let final_total = total_flag.load(std::sync::atomic::Ordering::Relaxed);
                            log::info!("Bulk thumbnail scan complete: {} files queued from {:?}", final_total, root);
                            ctx_clone.request_repaint();
                        })
                        .ok();
                }
                StatusBarAction::ShowHiddenChanged => {
                    app.save_preferences();
                    app.directory_cache.clear();
                    app.load_folder(true);
                }
                _ => {}
            }
        });
}
