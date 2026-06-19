use crate::app::ImageViewerApp;
use eframe::egui;
use rust_i18n::t;

pub(crate) fn render_status_bar_layer(app: &mut ImageViewerApp, ctx: &egui::Context) {
    // Detect bulk scan completion using bulk-specific counters instead of the shared queue.
    let is_scanning = app
        .bulk_thumbnail_scanning
        .load(std::sync::atomic::Ordering::Relaxed);
    let bulk_total = app
        .bulk_thumbnail_total
        .load(std::sync::atomic::Ordering::Relaxed);
    let bulk_completed = app
        .bulk_thumbnail_completed
        .load(std::sync::atomic::Ordering::Relaxed);
    let active_bulk_session = app
        .bulk_thumbnail_session
        .load(std::sync::atomic::Ordering::Relaxed);
    let bulk_progress_active = app
        .bulk_thumbnail_progress
        .lock()
        .ok()
        .and_then(|guard| {
            guard
                .as_ref()
                .map(|progress| progress.session == active_bulk_session)
        })
        .unwrap_or(false);
    let bulk_active =
        is_scanning || (bulk_progress_active && bulk_total > 0 && bulk_completed < bulk_total);

    if app.bulk_thumbnail_was_scanning
        && !is_scanning
        && bulk_progress_active
        && bulk_total > 0
        && bulk_completed >= bulk_total
    {
        app.notifications.push(
            crate::application::AppNotification::success(
                t!("status_bar.bulk_thumbnails_complete", count = bulk_total).to_string(),
            )
            .with_duration(std::time::Duration::from_secs(6)),
        );
        app.bulk_thumbnail_total
            .store(0, std::sync::atomic::Ordering::Relaxed);
        app.bulk_thumbnail_completed
            .store(0, std::sync::atomic::Ordering::Relaxed);
        crate::workers::thumbnail::clear_bulk_thumbnail_progress(&app.bulk_thumbnail_progress);
        app.bulk_thumbnail_was_scanning = false;
    } else if app.bulk_thumbnail_was_scanning
        && !is_scanning
        && (!bulk_progress_active || bulk_total == 0)
    {
        app.bulk_thumbnail_total
            .store(0, std::sync::atomic::Ordering::Relaxed);
        app.bulk_thumbnail_completed
            .store(0, std::sync::atomic::Ordering::Relaxed);
        crate::workers::thumbnail::clear_bulk_thumbnail_progress(&app.bulk_thumbnail_progress);
        app.bulk_thumbnail_was_scanning = false;
    } else {
        app.bulk_thumbnail_was_scanning = bulk_active;
    }

    if bulk_active || is_scanning {
        ctx.request_repaint(); // Keep UI refreshing while processing
    }

    egui::TopBottomPanel::bottom("status_bar")
        .exact_height(24.0)
        .show(ctx, |ui| {
            use crate::ui::status_bar::{render_status_bar, StatusBarAction};
            let video_preview_active = app
                .media_preview
                .as_ref()
                .map(|preview| {
                    preview.is_video() && preview.is_player_visible() && preview.is_visible()
                })
                .unwrap_or(false);
            let active_tag_filter_name = app
                .active_tag_filter
                .filter(|_| {
                    !app.navigation_state.is_computer_view
                        && !app.navigation_state.is_recycle_bin_view
                })
                .and_then(|id| app.tag_definitions.get(&id).map(|tag| tag.name.clone()));
            let action = render_status_bar(
                ui,
                &mut app.svg_icon_manager,
                &mut app.is_loading_folder,
                app.total_items,
                app.navigation_state.is_computer_view,
                app.navigation_state.is_recycle_bin_view,
                bulk_active || is_scanning,
                &mut app.show_hidden_files,
                app.view_mode,
                &mut app.thumbnail_size,
                video_preview_active,
                active_tag_filter_name.as_deref(),
            );
            match action {
                StatusBarAction::OpenSettings => {
                    app.navigation_state.show_settings_window = true;
                    app.navigation_state.active_settings_section =
                        crate::app::navigation_state::SettingsSection::General;
                }
                StatusBarAction::BulkThumbnailScan => {
                    if crate::domain::special_paths::is_virtual_path(
                        &app.navigation_state.current_path,
                    ) {
                        return;
                    }
                    let root = std::path::PathBuf::from(&app.navigation_state.current_path);
                    let queue = app.thumbnail_queue.clone();
                    let generation = app.generation;
                    let scanning_flag = app.bulk_thumbnail_scanning.clone();
                    let total_flag = app.bulk_thumbnail_total.clone();
                    let completed_flag = app.bulk_thumbnail_completed.clone();
                    let session_flag = app.bulk_thumbnail_session.clone();
                    let progress_state = app.bulk_thumbnail_progress.clone();
                    let ctx_clone = app.ui_ctx.clone();
                    let disk_cache = app.disk_cache.clone();
                    let bulk_session = session_flag
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                        .wrapping_add(1);
                    let is_virtual_drive =
                        crate::infrastructure::io_priority::is_virtual_drive_path(&root);

                    scanning_flag.store(true, std::sync::atomic::Ordering::Relaxed);
                    total_flag.store(0, std::sync::atomic::Ordering::Relaxed);
                    completed_flag.store(0, std::sync::atomic::Ordering::Relaxed);
                    crate::workers::thumbnail::begin_bulk_thumbnail_progress(
                        &progress_state,
                        &root,
                        bulk_session,
                    );
                    ctx.request_repaint();

                    std::thread::Builder::new()
                        .name("bulk-thumbnail-scan".into())
                        .spawn(move || {
                            use crate::infrastructure::windows::is_media_extension;
                            use crate::workers::thumbnail::ThumbnailPriority;

                            const MAX_BULK_THUMBNAIL_PENDING: usize = 4096;
                            const BULK_THUMBNAIL_BACKPRESSURE_SLEEP: std::time::Duration =
                                std::time::Duration::from_millis(8);

                            for entry in walkdir::WalkDir::new(&root)
                                .follow_links(false)
                                .into_iter()
                                .filter_map(|e| e.ok())
                            {
                                if session_flag.load(std::sync::atomic::Ordering::Relaxed)
                                    != bulk_session
                                    || !scanning_flag.load(std::sync::atomic::Ordering::Relaxed)
                                {
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
                                let modified = entry
                                    .metadata()
                                    .ok()
                                    .and_then(|m| m.modified().ok())
                                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                                if disk_cache
                                    .get(path, modified)
                                    .is_some_and(|entry| entry.satisfies_request(512))
                                {
                                    continue;
                                }

                                while queue.pending_count() >= MAX_BULK_THUMBNAIL_PENDING {
                                    if session_flag.load(std::sync::atomic::Ordering::Relaxed)
                                        != bulk_session
                                        || !scanning_flag.load(std::sync::atomic::Ordering::Relaxed)
                                    {
                                        break;
                                    }
                                    std::thread::sleep(BULK_THUMBNAIL_BACKPRESSURE_SLEEP);
                                }
                                if session_flag.load(std::sync::atomic::Ordering::Relaxed)
                                    != bulk_session
                                    || !scanning_flag.load(std::sync::atomic::Ordering::Relaxed)
                                {
                                    break;
                                }

                                crate::workers::thumbnail::set_bulk_thumbnail_current_file(
                                    &progress_state,
                                    path,
                                    bulk_session,
                                );
                                queue.push_bulk_scan(
                                    path.to_path_buf(),
                                    generation,
                                    512,
                                    ThumbnailPriority::Prefetch,
                                    modified
                                        .duration_since(std::time::SystemTime::UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_secs(),
                                    bulk_session,
                                );
                                if session_flag.load(std::sync::atomic::Ordering::Relaxed)
                                    != bulk_session
                                    || !scanning_flag.load(std::sync::atomic::Ordering::Relaxed)
                                {
                                    queue.cancel_bulk_scan_session(bulk_session);
                                    break;
                                }
                                total_flag.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                // Throttle traversal on virtual drives (Cryptomator/WinFsp)
                                // to reduce metadata I/O pressure on the FUSE driver.
                                if is_virtual_drive {
                                    std::thread::sleep(std::time::Duration::from_millis(2));
                                }
                            }
                            if session_flag.load(std::sync::atomic::Ordering::Relaxed)
                                == bulk_session
                            {
                                scanning_flag.store(false, std::sync::atomic::Ordering::Relaxed);
                            }
                            let final_total = total_flag.load(std::sync::atomic::Ordering::Relaxed);
                            log::info!(
                                "Bulk thumbnail scan complete: {} files queued from {:?}",
                                final_total,
                                root
                            );
                            ctx_clone.request_repaint();
                        })
                        .ok();
                }
                StatusBarAction::ShowHiddenChanged => {
                    app.save_preferences();
                    app.directory_cache.clear();
                    if let Some(tag_id) = crate::domain::special_paths::tag_id_from_view_path(
                        &app.navigation_state.current_path,
                    ) {
                        app.setup_tag_view(tag_id);
                    } else {
                        app.load_folder(true);
                    }
                    app.sidebar_tree.set_show_hidden(app.show_hidden_files);
                }
                StatusBarAction::ClearTagFilter => app.set_tag_filter(None),
                _ => {}
            }
        });
}
