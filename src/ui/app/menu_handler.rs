use crate::app::ImageViewerApp;
use eframe::egui;
use rust_i18n::t;
use std::path::{Path, PathBuf};

fn resolve_context_target_is_dir(
    app: &ImageViewerApp,
    item_idx: Option<usize>,
    path: &Path,
) -> bool {
    if crate::infrastructure::windows::is_drive_root_path(path) {
        return true;
    }

    if let Some(idx) = item_idx {
        if let Some(item) = app.items.get(idx) {
            if item.path == path {
                return item.is_dir || item.drive_info.is_some();
            }
        }
    }

    if let Some(selected) = app.selected_file.as_ref() {
        if selected.path == path {
            return selected.is_dir || selected.drive_info.is_some();
        }
    }

    if let Some(item) = app.items.iter().find(|item| item.path == path) {
        return item.is_dir || item.drive_info.is_some();
    }

    if app.pinned_folders.iter().any(|pinned| Path::new(&pinned.path) == path) {
        return true;
    }

    path == Path::new(&app.navigation_state.current_path)
}

fn is_onedrive_pin_text(text: &str) -> bool {
    let lower = text.trim().to_lowercase();
    lower.contains("always keep on this device") || lower.contains("sempre manter neste dispositivo")
}

fn is_onedrive_free_text(text: &str) -> bool {
    let lower = text.trim().to_lowercase();
    lower.contains("free up space") || lower.contains("liberar espaço")
}

fn onedrive_pin_command_from_text(text: &str) -> Option<crate::infrastructure::onedrive::PinCommand> {
    if is_onedrive_pin_text(text) {
        Some(crate::infrastructure::onedrive::PinCommand::AlwaysKeepOnDevice)
    } else if is_onedrive_free_text(text) {
        Some(crate::infrastructure::onedrive::PinCommand::FreeUpSpace)
    } else {
        None
    }
}

fn find_menu_item_text_by_id(
    items: &[crate::application::context_menu::ContextMenuItem],
    id: i32,
) -> Option<&str> {
    for item in items {
        if item.id == id {
            return Some(item.text.as_str());
        }

        if let Some(text) = find_menu_item_text_by_id(&item.sub_items, id) {
            return Some(text);
        }
    }

    None
}

pub fn handle_context_menu(app: &mut ImageViewerApp, ctx: &egui::Context) {
    // 1. Render the menu (ui construction)
    let mut context_menu = std::mem::take(&mut app.context_menu);

    let _ = crate::ui::context_menu::render_context_menu(
        ctx,
        &mut context_menu,
        &mut app.svg_icon_manager,
    );

    // 2. Handle lazy load request
    if let Some(id) = context_menu.pending_load_item.take() {
        app.context_menu = context_menu;
        app.handle_lazy_submenu_load(ctx, id);
        context_menu = std::mem::take(&mut app.context_menu);
    }

    // 3. Handle selected command before putting state back
    // CRITICAL: std::mem::take cleared app.context_menu, so internal commands
    // that call app.context_target_paths() would find empty target_paths and
    // fall back to selected_item/selected_file (wrong target). Restore them.
    app.context_menu.target_paths.clone_from(&context_menu.target_paths);

    if let Some(id) = context_menu.selected_command_id.take() {
        if id > 0 {
            // Shell command
            let selected_shell_item_text = find_menu_item_text_by_id(&context_menu.items, id);

            let direct_onedrive_pin_command = selected_shell_item_text
                .and_then(onedrive_pin_command_from_text);

            if let Some(command) = direct_onedrive_pin_command {
                let is_cloud_target = context_menu.target_paths.iter().any(|path| {
                    crate::infrastructure::onedrive::is_onedrive_path(path)
                        || crate::infrastructure::onedrive::path_has_cloud_attributes(path)
                });

                if is_cloud_target {
                    let mut had_error = false;
                    for path in &context_menu.target_paths {
                        if let Err(e) = crate::infrastructure::onedrive::set_pin_state(path, command) {
                            had_error = true;
                            log::warn!(
                                "[OneDrive] Failed to apply pin command {:?} to {:?}: {}",
                                command,
                                path,
                                e
                            );
                        }
                    }

                    if had_error {
                        app.notifications
                            .push(crate::application::AppNotification::error(
                                t!("operations.onedrive_command_failed").to_string(),
                            ));
                    }

                    app.directory_cache.invalidate(&std::path::PathBuf::from(&app.navigation_state.current_path));
                    // Invalidate each target and its children so navigation into
                    // affected folders reads fresh sync_status from disk, not cache.
                    for path in &context_menu.target_paths {
                        app.directory_cache.invalidate(path);
                        app.directory_cache.invalidate_children(path);
                    }
                    app.loaded_path.clear();
                    app.load_folder(false);
                    context_menu.close();
                    app.context_menu = context_menu;
                    return;
                }
            }

            if let Some(hwnd) = app.native_hwnd {
                // Dispatch to the worker thread — no blocking on the UI thread.
                let _ = app.shell_menu_req_tx.send(
                    crate::infrastructure::shell_menu_worker::ShellMenuRequest::Invoke {
                        command_id: id as u32,
                        menu_x: context_menu.position.x as i32,
                        menu_y: context_menu.position.y as i32,
                        hwnd_isize: hwnd.0 as isize,
                    },
                );

                // OneDrive pin fallback: apply the managed command in addition to
                // the shell invoke (some OneDrive shell extensions fire silently).
                if let Some(text) = selected_shell_item_text {
                    if let Some(command) = onedrive_pin_command_from_text(text) {
                        for path in &context_menu.target_paths {
                            let _ = crate::infrastructure::onedrive::set_pin_state(path, command);
                        }
                        app.directory_cache.invalidate(&std::path::PathBuf::from(
                            &app.navigation_state.current_path,
                        ));
                        // Same invalidation for the shell-invoke fallback path.
                        for path in &context_menu.target_paths {
                            app.directory_cache.invalidate(path);
                            app.directory_cache.invalidate_children(path);
                        }
                        app.loaded_path.clear();
                        app.load_folder(false);
                    }
                }
            }
        } else {
            // Internal command handled via trait
            let item_idx = context_menu.item_index;
            match id {
                -1 => app.create_new_folder(),
                -2 | -31 => app.command_copy(item_idx),
                -3 | -30 => app.command_cut(item_idx),
                -4 | -32 => app.command_paste(item_idx),
                -5 | -33 => {
                    if let Some(path) = context_menu.target_paths.first().cloned() {
                        if crate::infrastructure::windows::is_drive_root_path(&path) {
                            // Inline rename in sidebar — don't navigate to Este Computador
                            let drive_path_str = path.to_string_lossy().to_string();
                            let current_label = crate::infrastructure::windows::get_volume_label_raw(&drive_path_str)
                                .unwrap_or_default();
                            app.sidebar_renaming = Some((drive_path_str, current_label));
                            app.sidebar_rename_focus = true;
                        } else {
                            app.begin_rename_path(&path);
                        }
                    } else if let Some(idx) = item_idx.or(app.selected_item) {
                        app.begin_rename_item(idx);
                    }
                }
                -6 | -34 => {
                    if !context_menu.target_paths.is_empty() {
                        app.delete_with_shell_for_paths(&context_menu.target_paths);
                    }
                }
                -20 => {
                    if let Some(path) = app.context_target_paths(item_idx).first().cloned() {
                        if resolve_context_target_is_dir(app, item_idx, &path) {
                            app.navigate_to(&path.to_string_lossy());
                        } else {
                            app.open_with_shell_guarded(&path);
                        }
                    }
                }
                -21 => {
                    if let Some(path) = app.context_target_paths(item_idx).first().cloned() {
                        let target = if resolve_context_target_is_dir(app, item_idx, &path) {
                            path
                        } else {
                            path.parent()
                                .map(Path::to_path_buf)
                                .unwrap_or_else(|| PathBuf::from(&app.navigation_state.current_path))
                        };

                        let prev_view_mode = app.view_mode;
                        let prev_sort_mode = app.sort_mode;
                        let prev_sort_descending = app.sort_descending;
                        let prev_folders_position = app.folders_position;
                        app.sync_to_tab();
                        app.tab_manager.new_tab_at(&target.to_string_lossy());
                        let active = app.tab_manager.active_mut();
                        active.view_mode = prev_view_mode;
                        active.sort_mode = prev_sort_mode;
                        active.sort_descending = prev_sort_descending;
                        active.folders_position = prev_folders_position;
                        app.sync_from_tab();

                        if app.navigation_state.is_computer_view {
                            app.setup_computer_view();
                        } else {
                            app.watch_current_folder();
                            app.load_folder(false);
                        }
                    }
                }
                -24 => {
                    if let Some(path) = app.context_target_paths(item_idx).first().cloned() {
                        app.copy_path_to_clipboard(&path);
                    }
                }
                -26 => {
                    if let Some(path) = app.context_target_paths(item_idx).first().cloned() {
                        match app.create_shell_shortcut(&path) {
                            Ok(created) => {
                                app.load_folder(false);
                                app.notifications
                                    .push(crate::application::AppNotification::info(
                                        t!("operations.shortcut_created", name = created
                                            .file_name()
                                            .map(|n| n.to_string_lossy().to_string())
                                            .unwrap_or_default()).to_string(),
                                    ));
                            }
                            Err(e) => {
                                app.notifications
                                    .push(crate::application::AppNotification::error(
                                        t!("operations.shortcut_create_failed", error = e.to_string()).to_string(),
                                    ));
                            }
                        }
                    }
                }
                -28 => app.show_properties_for_idx(item_idx),
                -50 | -52 => {
                    if !context_menu.target_paths.is_empty() {
                        app.restore_from_recycle_bin(&context_menu.target_paths);
                    }
                }
                -51 | -53 => {
                    if !context_menu.target_paths.is_empty() {
                        app.delete_permanently(&context_menu.target_paths);
                    }
                }
                -54 => app.empty_recycle_bin(),
                -60 => {
                    // L-12: .to_string() breaks the Cow borrow before the mutable call
                    let path = app.context_target_paths(item_idx)
                        .first()
                        .and_then(|p| p.to_str())
                        .map(|s| s.to_string());
                    if let Some(path) = path {
                        app.pin_folder(&path);
                    }
                }
                -61 => {
                    let path = app.context_target_paths(item_idx)
                        .first()
                        .and_then(|p| p.to_str())
                        .map(|s| s.to_string());
                    if let Some(path) = path {
                        app.unpin_folder(&path);
                    }
                }
                _ => {}
            }
        }
        context_menu.close();
    } else if !context_menu.is_open {
        // Menu was dismissed without any command being invoked (Escape / click outside).
        // Tell the worker to release its COM context.
        let _ = app.shell_menu_req_tx.send(
            crate::infrastructure::shell_menu_worker::ShellMenuRequest::Cancel,
        );
        app.shell_menu_loading = false;
    }
    app.context_menu = context_menu;
}
