use eframe::egui;
use std::path::{Path, PathBuf};
use crate::app::ImageViewerApp;

pub fn handle_context_menu(app: &mut ImageViewerApp, ctx: &egui::Context) {
    // 1. Render the menu (ui construction)
    let mut context_menu = std::mem::take(&mut app.context_menu);
    let _ = crate::ui::context_menu::render_context_menu(
        ctx,
        &mut context_menu,
        &mut app.svg_icon_manager,
    );

    // 2. Handle selected command before putting state back
    if let Some(id) = context_menu.selected_command_id.take() {
        if id > 0 {
            // Shell command
            if let Some(native_ctx) = &context_menu.native_context {
                if let Some(shell_ctx) = native_ctx.downcast_ref::<crate::infrastructure::windows::native_menu::ShellMenuContext>() {
                    let _ = crate::infrastructure::windows::native_menu::invoke_menu_command(
                        app.native_hwnd.unwrap_or_default(),
                        &shell_ctx.context_menu,
                        id as u32,
                        context_menu.position.x as i32,
                        context_menu.position.y as i32,
                    );
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
                -6 | -34 => app.delete_with_shell_for_idx(item_idx),
                -20 => {
                    if let Some(path) = app.context_target_path(item_idx) {
                        if path.is_dir() {
                            app.navigate_to(&path.to_string_lossy());
                        } else {
                            open_with_shell(&path);
                        }
                    }
                }
                -21 => {
                    if let Some(path) = app.context_target_path(item_idx) {
                        let target = if path.is_dir() {
                            path
                        } else {
                            path.parent()
                                .map(Path::to_path_buf)
                                .unwrap_or_else(|| PathBuf::from(&app.current_path))
                        };

                        app.sync_to_tab();
                        app.tab_manager.new_tab_at(&target.to_string_lossy());
                        app.sync_from_tab();

                        if app.is_computer_view {
                            app.setup_computer_view();
                        } else {
                            app.watch_current_folder();
                            app.load_folder(false);
                        }
                    }
                }
                -24 => {
                    if let Some(path) = app.context_target_path(item_idx) {
                        app.copy_path_to_clipboard(&path);
                    }
                }
                -26 => {
                    if let Some(path) = app.context_target_path(item_idx) {
                        match app.create_shell_shortcut(&path) {
                            Ok(created) => {
                                app.load_folder(false);
                                app.notifications.push(
                                    crate::application::AppNotification::info(format!(
                                        "Atalho criado: {}",
                                        created.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()
                                    )),
                                );
                            }
                            Err(e) => {
                                app.notifications.push(
                                    crate::application::AppNotification::error(format!(
                                        "Falha ao criar atalho: {e}"
                                    )),
                                );
                            }
                        }
                    }
                }
                -28 => app.show_properties_for_idx(item_idx),
                -50 | -52 => {
                    if let Some(idx) = item_idx.or(app.selected_item) {
                        if let Some(item) = app.items.get(idx) {
                            let path = item.path.clone();
                            app.restore_from_recycle_bin(&path);
                        }
                    }
                }
                -51 | -53 => {
                    if let Some(idx) = item_idx.or(app.selected_item) {
                        if let Some(item) = app.items.get(idx) {
                            let path = item.path.clone();
                            app.delete_permanently(&path);
                        }
                    }
                }
                -54 => app.empty_recycle_bin(),
                _ => {}
            }
        }
        context_menu.close();
    }
    app.context_menu = context_menu;
}

fn open_with_shell(path: &Path) {
    let _ = std::process::Command::new("explorer").arg(path).spawn();
}
