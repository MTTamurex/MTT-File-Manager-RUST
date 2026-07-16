//! Context menu population
//!
//! This module handles population of the right-click context menu, merging native Shell items.

use crate::app::state::ImageViewerApp;
use eframe::egui;
use rust_i18n::t;
use std::path::PathBuf;

impl ImageViewerApp {
    pub fn context_target_paths<'a>(
        &'a self,
        item_idx: Option<usize>,
    ) -> std::borrow::Cow<'a, [std::path::PathBuf]> {
        // 1. Prioritize context menu state (populated by right-click)
        // L-12: Borrow the Vec instead of cloning — avoids allocation on the hot path.
        if !self.context_menu.target_paths.is_empty() {
            return std::borrow::Cow::Borrowed(&self.context_menu.target_paths);
        }

        // 2. Explicit item index
        if let Some(idx) = item_idx {
            if let Some(i) = self.items.get(idx) {
                return std::borrow::Cow::Owned(vec![i.path.clone()]);
            }
        }

        // 3. Multi-selection
        if !self.multi_selection.is_empty() {
            return std::borrow::Cow::Owned(self.multi_selection.iter().cloned().collect());
        }

        // 4. Single selection
        if let Some(sel) = &self.selected_file {
            return std::borrow::Cow::Owned(vec![sel.path.clone()]);
        }

        // 5. Current folder
        std::borrow::Cow::Owned(vec![std::path::PathBuf::from(
            &self.navigation_state.current_path,
        )])
    }

    pub fn can_open_empty_area_context_menu(&self) -> bool {
        !self.navigation_state.is_computer_view
            && crate::domain::special_paths::tag_id_from_view_path(
                &self.navigation_state.current_path,
            )
            .is_none()
    }

    pub fn populate_context_menu(
        &mut self,
        _ctx: &egui::Context,
        paths: &[PathBuf],
        is_empty_area: bool,
        _item_index: Option<usize>,
    ) {
        use crate::application::context_menu::ContextMenuItem;
        let is_global_search = self.context_menu.origin
            == crate::application::context_menu::ContextMenuOrigin::GlobalSearch;

        if !is_global_search && is_empty_area && !self.can_open_empty_area_context_menu() {
            self.context_menu.close();
            self.shell_menu_loading = false;
            return;
        }

        let drive_target_path = if !is_empty_area && paths.len() == 1 {
            let target = &paths[0];
            if crate::infrastructure::windows::is_drive_root_path(target) {
                Some(target.as_path())
            } else {
                None
            }
        } else {
            None
        };

        let mut items = Vec::new();

        // Special menu for Recycle Bin items
        if !is_global_search && self.navigation_state.is_recycle_bin_view && !is_empty_area {
            // Menu items for recycle bin (no primary icons)
            items.push(
                ContextMenuItem::new(-52, t!("context_menu.restore"))
                    .with_command("restore")
                    .with_svg_icon("refresh"),
            );
            items.push(
                ContextMenuItem::new(-53, t!("context_menu.delete_permanent"))
                    .with_command("delete_permanent")
                    .with_svg_icon("delete"),
            );
            items.push(ContextMenuItem::separator());
            items.push(
                ContextMenuItem::new(-28, t!("context_menu.properties"))
                    .with_command("properties")
                    .with_svg_icon("properties")
                    .with_shortcut(
                        self.shortcuts
                            .label(crate::app::shortcuts::ShortcutAction::Properties),
                    ),
            );

            self.context_menu.items = items;
            self.context_menu.partition_items(); // M-5
            return;
        }

        // Special menu for empty area in Recycle Bin
        if !is_global_search && self.navigation_state.is_recycle_bin_view && is_empty_area {
            items.push(
                ContextMenuItem::new(-54, t!("context_menu.empty_recycle_bin"))
                    .with_command("empty_recycle_bin")
                    .with_svg_icon("broom"),
            );
            self.context_menu.items = items;
            self.context_menu.partition_items(); // M-5
            return;
        }

        if !is_global_search && self.current_location_is_archive_namespace() {
            self.shell_menu_request_id = self.shell_menu_request_id.wrapping_add(1);
            let _ = self
                .shell_menu_req_tx
                .send(crate::infrastructure::shell_menu_worker::ShellMenuRequest::Cancel);

            if !is_empty_area {
                items.push(
                    ContextMenuItem::primary(-3, t!("context_menu.cut"))
                        .with_command("cut")
                        .with_shortcut(
                            self.shortcuts
                                .label(crate::app::shortcuts::ShortcutAction::Cut),
                        )
                        .enabled(false),
                );
                items.push(
                    ContextMenuItem::primary(-2, t!("context_menu.copy"))
                        .with_command("copy")
                        .with_shortcut(
                            self.shortcuts
                                .label(crate::app::shortcuts::ShortcutAction::Copy),
                        )
                        .enabled(self.can_copy_from_current_location()),
                );
                items.push(
                    ContextMenuItem::primary(-5, t!("context_menu.rename"))
                        .with_command("rename")
                        .with_shortcut(
                            self.shortcuts
                                .label(crate::app::shortcuts::ShortcutAction::Rename),
                        )
                        .enabled(false),
                );
                items.push(
                    ContextMenuItem::primary(-6, t!("context_menu.delete"))
                        .with_command("delete")
                        .with_shortcut(
                            self.shortcuts
                                .label(crate::app::shortcuts::ShortcutAction::Delete),
                        )
                        .enabled(false),
                );
            } else {
                items.push(
                    ContextMenuItem::primary(-4, t!("context_menu.paste"))
                        .with_command("paste")
                        .with_shortcut(
                            self.shortcuts
                                .label(crate::app::shortcuts::ShortcutAction::Paste),
                        )
                        .enabled(false),
                );
                items.push(ContextMenuItem::separator());
                items.push(
                    ContextMenuItem::new(-1, t!("context_menu.create_folder"))
                        .with_svg_icon("folder_new")
                        .with_shortcut(
                            self.shortcuts
                                .label(crate::app::shortcuts::ShortcutAction::CreateFolder),
                        )
                        .enabled(false),
                );
            }

            self.context_menu.items = items;
            self.context_menu.partition_items();
            self.shell_menu_loading = false;
            return;
        }

        // Check if the target item is a drive (drives don't support file operations)
        let is_drive = _item_index
            .and_then(|idx| self.items.get(idx))
            .map(|item| item.drive_info.is_some())
            .unwrap_or_else(|| drive_target_path.is_some());
        // Determine if the target is a file (not a folder, not a drive, not empty area).
        // Archives (.zip, .rar, .7z) have is_dir=true (they're navigable containers)
        // but still support "Open with" as files.
        // PE executables (.exe, .msi, .com, .scr) never show "Open with" in Windows Explorer.
        let target_is_file = if is_empty_area || is_drive {
            false
        } else if is_global_search {
            !self.context_menu.primary_is_directory.unwrap_or(false)
                && paths.first().is_some_and(|path| {
                    path.extension().is_none_or(|ext| {
                        !crate::domain::file_entry::is_executable_extension(&format!(
                            ".{}",
                            ext.to_string_lossy()
                        ))
                    })
                })
        } else if let Some(idx) = _item_index {
            self.items
                .get(idx)
                .map(|item| {
                    (!item.is_dir || item.is_archive())
                        && !crate::domain::file_entry::is_executable_extension(&item.name)
                })
                .unwrap_or(false)
        } else if let Some(path) = paths.first() {
            path.is_file()
                && path.extension().is_none_or(|ext| {
                    !crate::domain::file_entry::is_executable_extension(&format!(
                        ".{}",
                        ext.to_string_lossy()
                    ))
                })
        } else {
            false
        };
        let can_copy_target =
            !is_drive && (is_global_search || self.can_copy_from_current_location());
        let can_rename_target = if is_global_search {
            paths.len() == 1
                && !is_drive
                && paths.first().is_some_and(|path| {
                    !crate::domain::file_entry::path_contains_archive_segment(
                        &path.to_string_lossy().to_lowercase(),
                    )
                })
        } else if let Some(idx) = _item_index {
            self.can_rename_item(idx)
        } else if let Some(path) = drive_target_path {
            path.to_str().is_some_and(|drive_path| {
                crate::infrastructure::windows::drive_supports_volume_label_rename(
                    crate::infrastructure::windows::detect_drive_type(drive_path),
                )
            })
        } else {
            false
        };
        let paste_destination_is_archive = if is_empty_area {
            self.current_location_is_archive_namespace()
        } else {
            paths.first().is_some_and(|path| {
                self.context_target_is_directory(_item_index, path)
                    && Self::path_is_archive_namespace(path)
            })
        };
        let can_tag_targets = !is_empty_area
            && !is_drive
            && (is_global_search
                || (!self.navigation_state.is_computer_view
                    && !self.navigation_state.is_recycle_bin_view))
            && !paths.is_empty()
            && paths.iter().all(|path| {
                let path_text = path.to_string_lossy();
                let path_lower = path_text.to_lowercase();
                !path_text.starts_with("shell:")
                    && !crate::infrastructure::windows::is_drive_root_path(path)
                    && !crate::domain::file_entry::path_contains_archive_segment(&path_lower)
            });

        // ========== PRIMARY ITEMS (Header bar) - matching Files ==========
        // These appear as icon buttons in the header
        // Cut/Copy only make sense when an item is selected (not empty area)
        if !is_empty_area {
            items.push(
                ContextMenuItem::primary(-3, t!("context_menu.cut"))
                    .with_command("cut")
                    .with_shortcut(
                        self.shortcuts
                            .label(crate::app::shortcuts::ShortcutAction::Cut),
                    )
                    .enabled(!is_drive),
            );
            items.push(
                ContextMenuItem::primary(-2, t!("context_menu.copy"))
                    .with_command("copy")
                    .with_shortcut(
                        self.shortcuts
                            .label(crate::app::shortcuts::ShortcutAction::Copy),
                    )
                    .enabled(can_copy_target),
            );
        }

        if self.context_menu.origin.allows_paste() {
            let can_paste = self.can_paste_into_current_location() && !paste_destination_is_archive;
            items.push(
                ContextMenuItem::primary(-4, t!("context_menu.paste"))
                    .with_command("paste")
                    .with_shortcut(
                        self.shortcuts
                            .label(crate::app::shortcuts::ShortcutAction::Paste),
                    )
                    .enabled(can_paste && !is_drive),
            );
        }

        if !is_empty_area {
            items.push(
                ContextMenuItem::primary(-5, t!("context_menu.rename"))
                    .with_command("rename")
                    .with_shortcut(
                        self.shortcuts
                            .label(crate::app::shortcuts::ShortcutAction::Rename),
                    )
                    .enabled(can_rename_target),
            );
            items.push(
                ContextMenuItem::primary(-6, t!("context_menu.delete"))
                    .with_command("delete")
                    .with_shortcut(
                        self.shortcuts
                            .label(crate::app::shortcuts::ShortcutAction::Delete),
                    )
                    .enabled(!is_drive),
            );
        }
        // ========== SECONDARY ITEMS (App-specific) ==========
        let can_create_folder =
            !crate::domain::special_paths::is_virtual_path(&self.navigation_state.current_path)
                && !self.current_location_is_archive_namespace();
        if is_empty_area {
            items.push(ContextMenuItem::separator());
            items.push(
                ContextMenuItem::new(-1, t!("context_menu.create_folder"))
                    .with_svg_icon("folder_new")
                    .with_shortcut(
                        self.shortcuts
                            .label(crate::app::shortcuts::ShortcutAction::CreateFolder),
                    )
                    .enabled(can_create_folder),
            );
            items.push(
                ContextMenuItem::new(-80, t!("context_menu.open_terminal"))
                    .with_svg_icon("terminal"),
            );
            items.push(
                ContextMenuItem::new(-81, t!("context_menu.open_terminal_admin"))
                    .with_svg_icon("terminal"),
            );
        } else {
            items.push(ContextMenuItem::separator());
            items.push(ContextMenuItem::new(-20, t!("context_menu.open")).with_svg_icon("folder"));
            items.push(
                ContextMenuItem::new(-21, t!("context_menu.open_new_tab"))
                    .with_svg_icon("external-link"),
            );
            // Open with placeholder — only for files, inserted before shell items load
            if target_is_file {
                items.push(ContextMenuItem {
                    id: -201,
                    text: t!("context_menu.open_with").to_string(),
                    is_enabled: false,
                    is_loading_placeholder: true,
                    command_string: Some("openwith_placeholder".to_string()),
                    ..Default::default()
                });
            }
            items.push(
                ContextMenuItem::new(-80, t!("context_menu.open_terminal"))
                    .with_svg_icon("terminal"),
            );
            items.push(
                ContextMenuItem::new(-81, t!("context_menu.open_terminal_admin"))
                    .with_svg_icon("terminal"),
            );
            items.push(ContextMenuItem::separator());
            items.push(
                ContextMenuItem::new(-24, t!("context_menu.copy_path"))
                    .with_svg_icon("copy")
                    .with_shortcut("Ctrl+Shift+C"),
            );
            items.push(
                ContextMenuItem::new(-26, t!("context_menu.create_shortcut"))
                    .with_svg_icon("external-link"),
            );
            // Quick Access pin/unpin — only for folders (not drives)
            if !is_drive {
                if let Some(target_path) = paths.first().and_then(|p| p.to_str()) {
                    // Use cached is_dir field — avoids blocking I/O on OneDrive/network paths
                    let target_is_dir = self
                        .context_menu
                        .primary_is_directory
                        .or_else(|| {
                            _item_index
                                .and_then(|idx| self.items.get(idx))
                                .map(|item| item.is_dir)
                        })
                        .unwrap_or_else(|| {
                            // Fallback: search already-loaded items by path (no I/O)
                            self.items
                                .iter()
                                .find(|it| it.path.to_str() == Some(target_path))
                                .map(|it| it.is_dir)
                                .unwrap_or(false)
                        });
                    if target_is_dir {
                        let is_pinned = self.pinned_folders.iter().any(|pf| pf.path == target_path);
                        items.push(ContextMenuItem::separator());
                        if is_pinned {
                            items.push(
                                ContextMenuItem::new(-61, t!("context_menu.unpin_quick_access"))
                                    .with_svg_icon("pin"),
                            );
                        } else {
                            items.push(
                                ContextMenuItem::new(-60, t!("context_menu.pin_quick_access"))
                                    .with_svg_icon("pin"),
                            );
                        }
                    }
                }
            }

            // ========== CLOUD FILES ITEMS — "Always keep on this device" / "Free up space" ==========
            // Windows shell extensions for cloud files may not expose these items
            // through IContextMenu on newer Windows 11 builds, so we add them natively.
            if !is_drive {
                let cloud_sync = paths.first().and_then(|p| {
                    if !crate::infrastructure::onedrive::is_cloud_sync_path(p) {
                        return None;
                    }
                    // Use cached sync_status from already-loaded items (no I/O)
                    _item_index
                        .and_then(|idx| self.items.get(idx))
                        .map(|item| item.sync_status)
                        .or_else(|| {
                            self.items
                                .iter()
                                .find(|it| it.path == *p)
                                .map(|it| it.sync_status)
                        })
                });
                if let Some(status) = cloud_sync {
                    use crate::domain::file_entry::SyncStatus;
                    // Show "Always keep on this device" when NOT already pinned
                    // Show "Free up space" when NOT already cloud-only
                    let show_pin = status != SyncStatus::Pinned;
                    let show_free = status != SyncStatus::CloudOnly;
                    if show_pin || show_free {
                        items.push(ContextMenuItem::separator());
                        if show_pin {
                            items.push(
                                ContextMenuItem::new(-70, t!("context_menu.always_keep_on_device"))
                                    .with_command("onedrive_pin")
                                    .with_svg_icon("lock"),
                            );
                        }
                        if show_free {
                            items.push(
                                ContextMenuItem::new(-71, t!("context_menu.free_up_space"))
                                    .with_command("onedrive_free")
                                    .with_svg_icon("lock_open"),
                            );
                        }
                    }
                }
            }

            if can_tag_targets && !self.tag_definitions.is_empty() {
                let mut sub_items = Vec::new();
                for (idx, tag) in self.sorted_tag_definitions().into_iter().enumerate() {
                    sub_items.push(
                        ContextMenuItem::new(-9000 - idx as i32, tag.name.clone())
                            .with_command(format!("tag_toggle:{}", tag.id))
                            .with_leading_color(tag.color.to_color32())
                            .checked(self.paths_have_tag(paths, tag.id)),
                    );
                }
                sub_items.push(ContextMenuItem::separator());
                sub_items
                    .push(ContextMenuItem::new(-91, t!("tags.manage")).with_command("tag_manage"));

                items.push(ContextMenuItem::separator());
                items.push(
                    ContextMenuItem::new(-90, t!("tags.assign"))
                        .with_svg_icon("pin")
                        .with_subitems(sub_items),
                );
            }

            items.push(ContextMenuItem::separator());
            items.push(
                ContextMenuItem::new(-28, t!("context_menu.properties"))
                    .with_command("properties")
                    .with_svg_icon("properties")
                    .with_shortcut(
                        self.shortcuts
                            .label(crate::app::shortcuts::ShortcutAction::Properties),
                    ),
            );
        }

        // ========== SHELL ITEMS — extracted asynchronously on the worker thread ==========
        // Dispatch to the STA worker so Shell extensions cannot block the UI thread.
        // Results arrive via `shell_menu_res_rx`; the app polls them in its update loop
        // and calls `apply_async_shell_items` to merge them into `self.context_menu.items`.
        if let Some(hwnd) = self.native_hwnd {
            self.shell_menu_request_id = self.shell_menu_request_id.wrapping_add(1);
            let _ = self.shell_menu_req_tx.send(
                crate::infrastructure::shell_menu_worker::ShellMenuRequest::Extract {
                    request_id: self.shell_menu_request_id,
                    hwnd_isize: hwnd.0 as isize,
                    paths: paths.to_vec(),
                },
            );
            self.shell_menu_loading = true;

            // Add a single loading placeholder for "Show more options".
            // All shell items are placed inside this submenu, so only one
            // placeholder is needed and the menu height stays stable.
            items.push(ContextMenuItem::separator());
            items.push(ContextMenuItem {
                id: -200,
                text: t!("context_menu.show_more").to_string(),
                is_enabled: false,
                is_loading_placeholder: true,
                ..Default::default()
            });
        }

        self.context_menu.items = items;
        self.context_menu.partition_items(); // M-5
    }

    /// Convert `ShellMenuItemData` items received from the worker and merge them into
    /// the already-populated context menu.  Called from the update-loop polling code.
    pub fn apply_async_shell_items(
        &mut self,
        shell_items: Vec<crate::infrastructure::shell_menu_worker::ShellMenuItemData>,
        ctx: &egui::Context,
    ) {
        use crate::application::context_menu::ContextMenuItem;
        use crate::infrastructure::shell_menu_worker::ShellMenuItemData;
        use crate::infrastructure::windows::native_menu::is_known_verb;

        fn convert(ui_ctx: &egui::Context, item: &ShellMenuItemData) -> Option<ContextMenuItem> {
            // Filter verbs handled internally
            if let Some(ref verb) = item.command_string {
                if is_known_verb(verb) {
                    return None;
                }
            }
            // Text-based blacklist (localised strings)
            let lower = item.text.to_lowercase();
            const BLACKLIST: &[&str] = &[
                "pin to quick access",
                "fixar no acesso rápido",
                "restore previous versions",
                "restaurar versões anteriores",
                "copy as path",
                "copiar como caminho",
                "create shortcut",
                "criar atalho",
                // OneDrive items — handled natively to guarantee availability
                "always keep on this device",
                "sempre manter neste dispositivo",
                "free up space",
                "liberar espaço",
                // Terminal — handled natively via Open in Terminal command (-80, -81)
                "open in terminal",
                "abrir no terminal",
                "open in terminal (admin)",
                "abrir no terminal (admin)",
            ];
            if BLACKLIST.iter().any(|&t| lower.contains(t)) {
                return None;
            }

            let icon = item.icon_rgba.as_ref().map(|(rgba, w, h)| {
                ui_ctx.load_texture(
                    format!("menu_icon_{}", item.id),
                    egui::ColorImage::from_rgba_unmultiplied([*w as usize, *h as usize], rgba),
                    Default::default(),
                )
            });

            let sub_items = item
                .sub_items
                .iter()
                .filter_map(|s| convert(ui_ctx, s))
                .collect();

            Some(ContextMenuItem {
                id: item.id as i32,
                text: item.text.clone(),
                icon,
                sub_items,
                is_separator: item.is_separator,
                is_enabled: item.is_enabled,
                is_primary: false,
                keyboard_shortcut: None,
                command_string: item.command_string.clone(),
                show_in_overflow: false,
                has_pending_submenu: item.has_submenu,
                svg_icon_name: None,
                is_loading_placeholder: false,
                is_checked: false,
                leading_color: None,
            })
        }

        // Remove all loading placeholders before adding real items.
        // They were inserted in `populate_context_menu` to reserve space.
        self.context_menu
            .items
            .retain(|item| !item.is_loading_placeholder);
        // Remove any trailing separator(s) that preceded the placeholder block.
        while self
            .context_menu
            .items
            .last()
            .is_some_and(|item| item.is_separator)
        {
            self.context_menu.items.pop();
        }

        // Determine if the target is a file so we only promote "Open with" for files
        let target_is_file = self.context_menu.primary_is_directory.map_or_else(
            || {
                self.context_menu
                    .target_paths
                    .first()
                    .is_some_and(|p| p.is_file())
            },
            |is_directory| !is_directory,
        ) && self.context_menu.target_paths.first().is_some_and(|p| {
            p.extension().is_none_or(|ext| {
                !crate::domain::file_entry::is_executable_extension(&format!(
                    ".{}",
                    ext.to_string_lossy()
                ))
            })
        });

        let mut open_with_item: Option<ContextMenuItem> = None;
        let mut all_shell_items = Vec::new();

        for raw in &shell_items {
            if let Some(item) = convert(ctx, raw) {
                if item.is_separator {
                    continue;
                }
                // Promote "Open with" to the main menu only for files
                if target_is_file
                    && (item.text.to_lowercase().contains("open with")
                        || item.text.to_lowercase().contains("abrir com"))
                {
                    open_with_item = Some(item);
                } else {
                    all_shell_items.push(item);
                }
            }
        }

        let mut pending_open_with_submenu_load = None;
        let items = &mut self.context_menu.items;

        // Remove the Open with placeholder before inserting the real item
        if let Some(idx) = items
            .iter()
            .position(|i| i.command_string.as_deref() == Some("openwith_placeholder"))
        {
            items.remove(idx);
        }

        // Insert the shell "Open with" right after "Open in new tab" (-21)
        if let Some(mut open_with) = open_with_item {
            // Translate the text to match the current locale
            open_with.text = t!("context_menu.open_with").to_string();
            if open_with.has_pending_submenu && open_with.sub_items.is_empty() {
                pending_open_with_submenu_load = Some(open_with.id);
            }
            if let Some(idx) = items.iter().position(|i| i.id == -21) {
                items.insert(idx + 1, open_with);
            } else {
                // Fallback: append before the separator that precedes shell items
                items.push(open_with);
            }
        }

        if !all_shell_items.is_empty() {
            items.push(ContextMenuItem::separator());
            items.push(
                ContextMenuItem::new(-99, t!("context_menu.show_more"))
                    .with_subitems(all_shell_items),
            );
        }

        if let Some(id) = pending_open_with_submenu_load {
            self.context_menu.pending_load_item = Some(id);
        }

        self.context_menu.partition_items(); // M-5: re-partition after shell items are merged
        self.shell_menu_loading = false;
    }

    pub fn handle_lazy_submenu_load(&mut self, _egui_ctx: &egui::Context, item_id: i32) {
        // The ShellMenuContext now lives exclusively on the worker thread.
        // Send a LoadSubmenu request; the SubmenuLoaded response is processed in
        // the update-loop polling code which calls `apply_async_submenu_items`.
        let _ = self.shell_menu_req_tx.send(
            crate::infrastructure::shell_menu_worker::ShellMenuRequest::LoadSubmenu {
                request_id: self.shell_menu_request_id,
                item_id: item_id as u32,
            },
        );
        // Re-open the polling gate so the SubmenuLoaded response is picked up.
        self.shell_menu_loading = true;
    }

    /// Merge lazily-loaded submenu items (received from the worker) into the context menu tree.
    pub fn apply_async_submenu_items(
        &mut self,
        item_id: u32,
        sub_items: Vec<crate::infrastructure::shell_menu_worker::ShellMenuItemData>,
        ctx: &egui::Context,
    ) {
        use crate::application::context_menu::ContextMenuItem;
        use crate::infrastructure::shell_menu_worker::ShellMenuItemData;

        fn convert_item(ui_ctx: &egui::Context, item: &ShellMenuItemData) -> ContextMenuItem {
            let icon = item.icon_rgba.as_ref().map(|(rgba, w, h)| {
                ui_ctx.load_texture(
                    format!("menu_icon_{}", item.id),
                    egui::ColorImage::from_rgba_unmultiplied([*w as usize, *h as usize], rgba),
                    Default::default(),
                )
            });
            ContextMenuItem {
                id: item.id as i32,
                text: item.text.clone(),
                icon,
                sub_items: item
                    .sub_items
                    .iter()
                    .map(|s| convert_item(ui_ctx, s))
                    .collect(),
                is_separator: item.is_separator,
                is_enabled: item.is_enabled,
                is_primary: false,
                keyboard_shortcut: None,
                command_string: item.command_string.clone(),
                show_in_overflow: false,
                has_pending_submenu: item.has_submenu,
                svg_icon_name: None,
                is_loading_placeholder: false,
                is_checked: false,
                leading_color: None,
            }
        }

        fn update_ui_item(
            items: &mut [ContextMenuItem],
            id: i32,
            new_subitems: Vec<ContextMenuItem>,
        ) -> bool {
            for item in items {
                if item.id == id {
                    item.sub_items = new_subitems;
                    item.has_pending_submenu = false;
                    return true;
                }
                if update_ui_item(&mut item.sub_items, id, new_subitems.clone()) {
                    return true;
                }
            }
            false
        }

        let new_subitems: Vec<ContextMenuItem> =
            sub_items.iter().map(|s| convert_item(ctx, s)).collect();
        update_ui_item(&mut self.context_menu.items, item_id as i32, new_subitems);
    }
}
