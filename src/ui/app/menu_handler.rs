use crate::app::ImageViewerApp;
use eframe::egui;
use rust_i18n::t;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::HWND,
        UI::Shell::{ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SHELLEXECUTEINFOW},
        UI::WindowsAndMessaging::SW_SHOWNORMAL,
    },
};

/// Launches a terminal in the given directory.
/// Tries Windows Terminal (`wt.exe`) first; falls back to PowerShell.
fn open_terminal_at(path: &Path) {
    let dir = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| path.to_path_buf())
    };

    if std::process::Command::new("wt.exe")
        .arg("-d")
        .arg(&dir)
        .spawn()
        .is_err()
    {
        let _ = std::process::Command::new("powershell.exe")
            .arg("-NoExit")
            .current_dir(&dir)
            .spawn();
    }
}

/// Spawn a program elevated via UAC using `ShellExecuteExW` with the `"runas"` verb.
/// Returns `true` if the elevated process was launched successfully.
fn elevated_spawn(program: &str, args: &[&str]) -> bool {
    let program_wide: Vec<u16> = program.encode_utf16().chain(std::iter::once(0)).collect();
    let verb_wide: Vec<u16> = "runas".encode_utf16().chain(std::iter::once(0)).collect();

    // Build a single parameter string, quoting each argument.
    let params: String = args
        .iter()
        .map(|a| {
            if a.contains(' ') {
                format!("\"{}\"", a)
            } else {
                (*a).to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    let params_wide: Vec<u16> = params.encode_utf16().chain(std::iter::once(0)).collect();

    let mut exec_info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_FLAG_NO_UI,
        hwnd: HWND::default(),
        lpVerb: PCWSTR(verb_wide.as_ptr()),
        lpFile: PCWSTR(program_wide.as_ptr()),
        lpParameters: PCWSTR(params_wide.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };

    unsafe { ShellExecuteExW(&mut exec_info).is_ok() }
}

/// Launches an elevated terminal (UAC prompt) in the given directory.
/// Tries Windows Terminal (`wt.exe`) first; falls back to PowerShell.
fn open_terminal_admin_at(path: &Path) {
    let dir = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| path.to_path_buf())
    };

    let dir_str = dir.to_string_lossy();
    if !elevated_spawn("wt.exe", &["-d", &dir_str]) {
        let cd_cmd = format!("cd '{}'", dir.display());
        elevated_spawn("powershell.exe", &["-NoExit", "-Command", &cd_cmd]);
    }
}

fn is_cloud_files_pin_text(text: &str) -> bool {
    let lower = text.trim().to_lowercase();
    lower.contains("always keep on this device")
        || lower.contains("sempre manter neste dispositivo")
}

fn is_cloud_files_free_text(text: &str) -> bool {
    let lower = text.trim().to_lowercase();
    lower.contains("free up space") || lower.contains("liberar espaço")
}

fn cloud_files_pin_command_from_text(
    text: &str,
) -> Option<crate::infrastructure::onedrive::PinCommand> {
    if is_cloud_files_pin_text(text) {
        Some(crate::infrastructure::onedrive::PinCommand::AlwaysKeepOnDevice)
    } else if is_cloud_files_free_text(text) {
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

fn find_menu_item_command_by_id(
    items: &[crate::application::context_menu::ContextMenuItem],
    id: i32,
) -> Option<&str> {
    for item in items {
        if item.id == id {
            return item.command_string.as_deref();
        }

        if let Some(command) = find_menu_item_command_by_id(&item.sub_items, id) {
            return Some(command);
        }
    }

    None
}

fn apply_cloud_files_pin(
    app: &mut ImageViewerApp,
    target_paths: &[PathBuf],
    command: crate::infrastructure::onedrive::PinCommand,
) {
    let paths = target_paths.to_vec();
    let ui_ctx = app.ui_ctx.clone();
    let reload_flag = Arc::clone(&app.onedrive_pin_reload_pending);
    let dir_cache = Arc::clone(&app.directory_cache);
    let dirty_reg = Arc::clone(&app.directory_dirty_registry);
    let current_dir = PathBuf::from(&app.navigation_state.current_path);

    // Run the blocking attrib commands on a background thread.
    std::thread::spawn(move || {
        for path in &paths {
            if let Err(e) = crate::infrastructure::onedrive::set_pin_state(path, command) {
                log::warn!(
                    "[CloudFiles] Failed to apply pin command {:?} to {:?}: {}",
                    command,
                    path,
                    e
                );
            }
        }
        // Invalidate caches AFTER attrib finishes so the next read gets fresh data.
        dir_cache.invalidate(&current_dir);
        for path in &paths {
            dir_cache.invalidate(path);
            dir_cache.invalidate_children(path);
            dirty_reg.mark_dirty(path);
        }
        dirty_reg.mark_dirty(&current_dir);
        // Signal the UI thread to reload the folder.
        reload_flag.store(true, std::sync::atomic::Ordering::Release);
        ui_ctx.request_repaint();
    });
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
    app.context_menu
        .target_paths
        .clone_from(&context_menu.target_paths);

    if let Some(id) = context_menu.selected_command_id.take() {
        if id > 0 {
            // Shell command
            let selected_shell_item_text = find_menu_item_text_by_id(&context_menu.items, id);

            let direct_cloud_files_pin_command =
                selected_shell_item_text.and_then(cloud_files_pin_command_from_text);

            if let Some(command) = direct_cloud_files_pin_command {
                let is_cloud_target = context_menu
                    .target_paths
                    .iter()
                    .any(|path| crate::infrastructure::onedrive::is_cloud_sync_path(path));

                if is_cloud_target {
                    apply_cloud_files_pin(app, &context_menu.target_paths, command);
                    context_menu.close();
                    app.context_menu = context_menu;
                    return;
                }
            }

            // Handle "Open with" natively — ShellExecuteExW with "openas" is more
            // reliable than IContextMenu::InvokeCommand for this specific verb.
            let is_open_with = selected_shell_item_text.map_or(false, |text| {
                let lower = text.to_lowercase();
                lower.contains("open with") || lower.contains("abrir com")
            });
            if is_open_with {
                if let Some(path) = context_menu.target_paths.first() {
                    if let Some(hwnd) = app.native_hwnd {
                        if let Err(e) =
                            crate::application::file_operations::open_with_dialog(path, Some(hwnd))
                        {
                            log::warn!("Open with dialog failed for '{}': {}", path.display(), e);
                        }
                    }
                }
                context_menu.close();
                app.context_menu = context_menu;
                return;
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

                // Cloud Files pin fallback: apply the managed command in addition to
                // the shell invoke (some shell extensions fire silently).
                if let Some(text) = selected_shell_item_text {
                    if let Some(command) = cloud_files_pin_command_from_text(text) {
                        if context_menu
                            .target_paths
                            .iter()
                            .any(|path| crate::infrastructure::onedrive::is_cloud_sync_path(path))
                        {
                            apply_cloud_files_pin(app, &context_menu.target_paths, command);
                        }
                    }
                }
            }
        } else {
            // Internal command handled via trait
            let item_idx = context_menu.item_index;
            let selected_command = find_menu_item_command_by_id(&context_menu.items, id)
                .map(|command| command.to_string());
            if let Some(command) = selected_command.as_deref() {
                if let Some(tag_id_raw) = command.strip_prefix("tag_toggle:") {
                    if let Ok(tag_id) = tag_id_raw.parse::<i64>() {
                        app.toggle_tag_on_paths(&context_menu.target_paths, tag_id);
                    }
                    context_menu.close();
                    app.context_menu = context_menu;
                    return;
                }
                if command == "tag_manage" {
                    app.show_tag_manager = true;
                    context_menu.close();
                    app.context_menu = context_menu;
                    return;
                }
            }
            match id {
                -1 => app.create_new_folder(),
                -2 | -31 => app.command_copy(item_idx),
                -3 | -30 => app.command_cut(item_idx),
                -4 | -32 => app.command_paste(item_idx),
                -5 | -33 => {
                    if let Some(path) = context_menu.target_paths.first().cloned() {
                        if crate::infrastructure::windows::is_drive_root_path(&path) {
                            // Inline rename in sidebar — don't navigate to Este Computador
                            let drive_path_str = path.to_string_lossy();
                            let current_label =
                                crate::infrastructure::windows::get_volume_label_raw(
                                    drive_path_str.as_ref(),
                                )
                                .unwrap_or_default();
                            app.sidebar_renaming =
                                Some((drive_path_str.into_owned(), current_label));
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
                        if app.context_target_is_directory(item_idx, &path) {
                            let target = path.to_string_lossy();
                            app.navigate_to(target.as_ref());
                        } else {
                            app.open_with_shell_guarded(&path);
                        }
                    }
                }
                -21 => {
                    if let Some(path) = app.context_target_paths(item_idx).first().cloned() {
                        let target = if app.context_target_is_directory(item_idx, &path) {
                            path
                        } else {
                            path.parent().map(Path::to_path_buf).unwrap_or_else(|| {
                                PathBuf::from(&app.navigation_state.current_path)
                            })
                        };

                        let prev_view_mode = app.view_mode;
                        let prev_sort_mode = app.sort_mode;
                        let prev_sort_descending = app.sort_descending;
                        let prev_folders_position = app.folders_position;
                        app.sync_to_tab();
                        let target_str = target.to_string_lossy();
                        app.tab_manager.new_tab_at(target_str.as_ref());
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
                                        t!(
                                            "operations.shortcut_created",
                                            name = created
                                                .file_name()
                                                .map(|n| n.to_string_lossy().to_string())
                                                .unwrap_or_default()
                                        )
                                        .to_string(),
                                    ));
                            }
                            Err(e) => {
                                app.notifications
                                    .push(crate::application::AppNotification::error(
                                        t!(
                                            "operations.shortcut_create_failed",
                                            error = e.to_string()
                                        )
                                        .to_string(),
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
                    let path = app
                        .context_target_paths(item_idx)
                        .first()
                        .and_then(|p| p.to_str())
                        .map(|s| s.to_string());
                    if let Some(path) = path {
                        app.pin_folder(&path);
                    }
                }
                -61 => {
                    let path = app
                        .context_target_paths(item_idx)
                        .first()
                        .and_then(|p| p.to_str())
                        .map(|s| s.to_string());
                    if let Some(path) = path {
                        app.unpin_folder(&path);
                    }
                }
                // Cloud Files: "Always keep on this device"
                -70 => {
                    apply_cloud_files_pin(
                        app,
                        &context_menu.target_paths,
                        crate::infrastructure::onedrive::PinCommand::AlwaysKeepOnDevice,
                    );
                }
                // Cloud Files: "Free up space"
                -71 => {
                    apply_cloud_files_pin(
                        app,
                        &context_menu.target_paths,
                        crate::infrastructure::onedrive::PinCommand::FreeUpSpace,
                    );
                }
                -80 => {
                    let path = if context_menu.is_empty_area {
                        PathBuf::from(&app.navigation_state.current_path)
                    } else {
                        context_menu
                            .target_paths
                            .first()
                            .cloned()
                            .unwrap_or_else(|| PathBuf::from(&app.navigation_state.current_path))
                    };
                    open_terminal_at(&path);
                }
                -81 => {
                    let path = if context_menu.is_empty_area {
                        PathBuf::from(&app.navigation_state.current_path)
                    } else {
                        context_menu
                            .target_paths
                            .first()
                            .cloned()
                            .unwrap_or_else(|| PathBuf::from(&app.navigation_state.current_path))
                    };
                    open_terminal_admin_at(&path);
                }
                -90 => {}
                -91 => {
                    app.show_tag_manager = true;
                }
                _ => {}
            }
        }
        context_menu.close();
    } else if !context_menu.is_open {
        // Menu was dismissed without any command being invoked (Escape / click outside).
        // Tell the worker to release its COM context.
        let _ = app
            .shell_menu_req_tx
            .send(crate::infrastructure::shell_menu_worker::ShellMenuRequest::Cancel);
        app.shell_menu_loading = false;
    }
    app.context_menu = context_menu;
}
