use crate::app::ImageViewerApp;
use eframe::egui;
use std::path::{Path, PathBuf};

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
) -> Option<String> {
    for item in items {
        if item.id == id {
            return Some(item.text.clone());
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
    let target_paths = context_menu.target_paths.clone(); // PRESERVE PATHS

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
    if let Some(id) = context_menu.selected_command_id.take() {
        if id > 0 {
            // Shell command
            let selected_shell_item_text = find_menu_item_text_by_id(&context_menu.items, id);

            let direct_onedrive_pin_command = selected_shell_item_text
                .as_deref()
                .and_then(onedrive_pin_command_from_text);

            if let Some(command) = direct_onedrive_pin_command {
                let is_cloud_target = target_paths.iter().any(|path| {
                    crate::infrastructure::onedrive::is_onedrive_path(path)
                        || crate::infrastructure::onedrive::path_has_cloud_attributes(path)
                });

                if is_cloud_target {
                    let mut had_error = false;
                    for path in &target_paths {
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
                                "Falha ao aplicar comando do OneDrive em um ou mais itens".to_string(),
                            ));
                    }

                    app.directory_cache.invalidate(&std::path::PathBuf::from(&app.navigation_state.current_path));
                    app.loaded_path.clear();
                    app.load_folder(false);
                    context_menu.close();
                    app.context_menu = context_menu;
                    return;
                }
            }

            if let Some(native_ctx) = &context_menu.native_context {
                if let Some(shell_ctx) = native_ctx.downcast_ref::<crate::infrastructure::windows::native_menu::ShellMenuContext>() {
                    let shell_result = crate::infrastructure::windows::native_menu::invoke_menu_command(
                        app.native_hwnd.unwrap_or_default(),
                        &shell_ctx.context_menu,
                        id as u32,
                        context_menu.position.x as i32,
                        context_menu.position.y as i32,
                    );

                    // Fallback for OneDrive pin-state commands when shell invoke fails silently.
                    if shell_result.is_err() {
                        if let Some(text) = selected_shell_item_text.as_deref() {
                            let command = onedrive_pin_command_from_text(text);

                            if let Some(command) = command {
                                for path in &target_paths {
                                    let _ = crate::infrastructure::onedrive::set_pin_state(path, command);
                                }
                                app.directory_cache.invalidate(&std::path::PathBuf::from(&app.navigation_state.current_path));
                                app.loaded_path.clear();
                                app.load_folder(false);
                            }
                        }
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
                    if let Some(idx) = item_idx.or(app.selected_item) {
                        if let Some(item) = app.items.get(idx) {
                            app.renaming_state = Some((idx, item.name.clone()));
                            app.focus_rename = true;
                        }
                    }
                }
                -6 | -34 => {
                    if !target_paths.is_empty() {
                        app.delete_with_shell_for_paths(&target_paths);
                    }
                }
                -20 => {
                    if let Some(path) = app.context_target_paths(item_idx).first().cloned() {
                        if path.is_dir() {
                            app.navigate_to(&path.to_string_lossy());
                        } else {
                            app.open_with_shell_guarded(&path);
                        }
                    }
                }
                -21 => {
                    if let Some(path) = app.context_target_paths(item_idx).first().cloned() {
                        let target = if path.is_dir() {
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
                                    .push(crate::application::AppNotification::info(format!(
                                        "Atalho criado: {}",
                                        created
                                            .file_name()
                                            .map(|n| n.to_string_lossy())
                                            .unwrap_or_default()
                                    )));
                            }
                            Err(e) => {
                                app.notifications
                                    .push(crate::application::AppNotification::error(format!(
                                        "Falha ao criar atalho: {e}"
                                    )));
                            }
                        }
                    }
                }
                -28 => app.show_properties_for_idx(item_idx),
                -50 | -52 => {
                    if !target_paths.is_empty() {
                        app.restore_from_recycle_bin(&target_paths);
                    }
                }
                -51 | -53 => {
                    if !target_paths.is_empty() {
                        app.delete_permanently(&target_paths);
                    }
                }
                -54 => app.empty_recycle_bin(),
                -60 => {
                    if let Some(path) = app.context_target_paths(item_idx).first()
                        .and_then(|p| p.to_str())
                    {
                        app.pin_folder(path);
                    }
                }
                -61 => {
                    if let Some(path) = app.context_target_paths(item_idx).first()
                        .and_then(|p| p.to_str())
                    {
                        app.unpin_folder(path);
                    }
                }
                _ => {}
            }
        }
        context_menu.close();
    }
    app.context_menu = context_menu;
}
