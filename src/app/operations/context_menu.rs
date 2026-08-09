//! Context menu population
//!
//! This module handles population of the right-click context menu, merging native Shell items.

use crate::app::state::ImageViewerApp;
use eframe::egui;
use rust_i18n::t;
use std::path::PathBuf;
use std::sync::atomic::Ordering;

fn is_optical_disc_context_target(
    is_empty_area: bool,
    path_count: usize,
    is_drive_root: bool,
    drive_type: Option<crate::infrastructure::windows::DriveType>,
) -> bool {
    !is_empty_area
        && path_count == 1
        && is_drive_root
        && drive_type == Some(crate::infrastructure::windows::DriveType::Cdrom)
}

fn is_open_with_menu_item(item: &crate::application::context_menu::ContextMenuItem) -> bool {
    item.command_string.as_deref().is_some_and(|command| {
        matches!(
            command,
            "openwith_placeholder"
                | "open_with_menu"
                | "open_with_dialog"
                | "open_with_shell_fallback"
        ) || command.eq_ignore_ascii_case("openas")
    })
}

fn is_extract_all_shell_item(item: &crate::application::context_menu::ContextMenuItem) -> bool {
    if item
        .command_string
        .as_deref()
        .is_some_and(|verb| verb.eq_ignore_ascii_case("extractall"))
    {
        return true;
    }

    let text = item.text.trim().trim_end_matches(['.', '\u{2026}']).trim();
    text.eq_ignore_ascii_case("extract all") || text.eq_ignore_ascii_case("extrair tudo")
}

fn is_extract_all_pending_item(item: &crate::application::context_menu::ContextMenuItem) -> bool {
    item.command_string.as_deref()
        == Some(crate::application::context_menu::EXTRACT_ALL_PENDING_COMMAND)
}

fn should_offer_extract_all(paths: &[PathBuf], is_empty_area: bool, target_is_file: bool) -> bool {
    !is_empty_area
        && target_is_file
        && paths.len() == 1
        && paths.first().is_some_and(|path| {
            crate::domain::file_entry::is_archive_extension(&path.to_string_lossy())
        })
}

fn insert_extract_all_pending_item(
    menu_items: &mut Vec<crate::application::context_menu::ContextMenuItem>,
) {
    use crate::application::context_menu::{
        ContextMenuItem, EXTRACT_ALL_PENDING_COMMAND, EXTRACT_ALL_PENDING_ID,
    };

    if menu_items.iter().any(is_extract_all_pending_item) {
        return;
    }
    let Some(open_with_index) = menu_items.iter().position(is_open_with_menu_item) else {
        return;
    };

    menu_items.insert(
        open_with_index + 1,
        ContextMenuItem::new(EXTRACT_ALL_PENDING_ID, t!("context_menu.extract_all"))
            .with_command(EXTRACT_ALL_PENDING_COMMAND)
            .with_svg_icon("extract_all"),
    );
}

pub(crate) fn extract_all_shell_command_id(
    menu_items: &[crate::application::context_menu::ContextMenuItem],
) -> Option<i32> {
    menu_items
        .iter()
        .find(|item| {
            item.id > 0
                && item.is_enabled
                && !item.is_separator
                && item.sub_items.is_empty()
                && is_extract_all_shell_item(item)
        })
        .map(|item| item.id)
}

fn promote_extract_all_shell_item(
    menu_items: &mut Vec<crate::application::context_menu::ContextMenuItem>,
    shell_items: &mut Vec<crate::application::context_menu::ContextMenuItem>,
) {
    let pending_index = menu_items.iter().position(is_extract_all_pending_item);
    let Some(extract_all_index) = shell_items.iter().position(is_extract_all_shell_item) else {
        if let Some(index) = pending_index {
            menu_items.remove(index);
        }
        return;
    };

    if let Some(index) = pending_index {
        let mut extract_all = shell_items.remove(extract_all_index);
        extract_all.svg_icon_name = Some("extract_all".to_string());
        menu_items[index] = extract_all;
        return;
    }

    let Some(open_with_index) = menu_items.iter().position(is_open_with_menu_item) else {
        return;
    };

    let mut extract_all = shell_items.remove(extract_all_index);
    extract_all.svg_icon_name = Some("extract_all".to_string());
    menu_items.insert(open_with_index + 1, extract_all);
}

fn grouping_menu_item(app: &ImageViewerApp) -> crate::application::context_menu::ContextMenuItem {
    use crate::application::context_menu::ContextMenuItem;
    use crate::domain::file_entry::{GroupMode, ViewMode};

    let mut sub_items = Vec::new();
    for (index, mode, label) in [
        (0, GroupMode::None, t!("secondary_toolbar.group_none")),
        (1, GroupMode::Name, t!("secondary_toolbar.group_name")),
        (2, GroupMode::Date, t!("secondary_toolbar.group_date")),
        (3, GroupMode::Type, t!("secondary_toolbar.group_type")),
        (4, GroupMode::Size, t!("secondary_toolbar.group_size")),
    ] {
        sub_items.push(
            ContextMenuItem::new(-300 - index, label)
                .with_command(format!("group:{}", mode.preference_value()))
                .checked(app.group_mode == mode),
        );
    }
    sub_items.push(ContextMenuItem::separator());
    sub_items.push(
        ContextMenuItem::new(-306, t!("grouping.ascending"))
            .with_command("group_direction:ascending")
            .checked(!app.group_descending)
            .enabled(app.group_mode != GroupMode::None),
    );
    sub_items.push(
        ContextMenuItem::new(-307, t!("grouping.descending"))
            .with_command("group_direction:descending")
            .checked(app.group_descending)
            .enabled(app.group_mode != GroupMode::None),
    );

    ContextMenuItem::new(-299, t!("secondary_toolbar.group_by"))
        .with_subitems(sub_items)
        .enabled(
            !app.current_folder_locked
                && !app.navigation_state.is_computer_view
                && app.view_mode != ViewMode::Miller,
        )
}

impl ImageViewerApp {
    pub(crate) fn invalidate_context_menu_workers(&mut self) {
        self.shell_menu_request_id = self.shell_menu_request_id.wrapping_add(1);
        self.latest_shell_menu_request_id
            .store(self.shell_menu_request_id, Ordering::Release);
        self.pending_shell_menu_invocation_id
            .store(0, Ordering::Release);
        self.pending_open_with_invocation_id
            .store(0, Ordering::Release);
        let _ = self
            .shell_menu_control_tx
            .try_send(crate::infrastructure::shell_menu_worker::ShellMenuRequest::Cancel);
        let _ = self
            .open_with_control_tx
            .try_send(crate::infrastructure::open_with_worker::OpenWithRequest::Cancel);
        self.shell_menu_loading = false;
        self.open_with_loading = false;
        self.context_menu_workers_active = false;
    }

    pub(crate) fn supersede_context_menu_background_work(&self) {
        self.latest_shell_menu_request_id.store(
            self.shell_menu_request_id.wrapping_add(1),
            Ordering::Release,
        );
    }

    pub(crate) fn capture_context_menu_panel_origin(&mut self) {
        let panel = if self.in_inactive_panel_context {
            self.dual_panel_active.other()
        } else {
            self.dual_panel_active
        };
        self.context_menu.origin_panel_is_left =
            Some(panel == crate::app::dual_panel::ActivePanel::Left);
    }

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
    }

    pub fn dismiss_context_menu(&mut self) {
        if !self.context_menu.is_open {
            return;
        }

        self.context_menu.close();
        self.invalidate_context_menu_workers();
    }

    pub fn populate_context_menu(
        &mut self,
        _ctx: &egui::Context,
        paths: &[PathBuf],
        is_empty_area: bool,
        _item_index: Option<usize>,
    ) {
        use crate::application::context_menu::ContextMenuItem;
        self.invalidate_context_menu_workers();
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

        if !is_global_search
            && is_empty_area
            && (self.navigation_state.is_computer_view
                || crate::domain::special_paths::tag_id_from_view_path(
                    &self.navigation_state.current_path,
                )
                .is_some())
        {
            items.push(grouping_menu_item(self));
            self.context_menu.items = items;
            self.context_menu.partition_items();
            return;
        }

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
            items.push(ContextMenuItem::separator());
            items.push(grouping_menu_item(self));
            self.context_menu.items = items;
            self.context_menu.partition_items(); // M-5
            return;
        }

        let explicit_operation_location = self
            .context_menu
            .primary_is_directory
            .and_then(|_| paths.first())
            .and_then(|path| {
                if is_empty_area {
                    Some(path.as_path())
                } else {
                    path.parent()
                }
            });
        let operation_location_is_archive = explicit_operation_location
            .is_some_and(Self::path_is_archive_namespace)
            || (explicit_operation_location.is_none()
                && self.current_location_is_archive_namespace());

        if !is_global_search && operation_location_is_archive {
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
                items.push(ContextMenuItem::separator());
                items.push(grouping_menu_item(self));
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
        let target_drive_type = _item_index
            .and_then(|idx| self.items.get(idx))
            .and_then(|item| item.drive_info.as_ref())
            .map(|info| info.drive_type)
            .or_else(|| {
                drive_target_path.map(|path| {
                    crate::infrastructure::windows::detect_drive_type(
                        path.to_string_lossy().as_ref(),
                    )
                })
            });
        let can_play_optical_disc = is_optical_disc_context_target(
            is_empty_area,
            paths.len(),
            drive_target_path.is_some(),
            target_drive_type,
        );
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
        let can_cut_target = !is_drive
            && paths.iter().all(|path| {
                !crate::domain::file_entry::is_path_inside_existing_archive_file(path)
                    && !self.path_is_same_or_ancestor_of_open_panel(path)
            });
        let can_delete_target = !is_drive
            && paths
                .iter()
                .all(|path| !self.path_is_same_or_ancestor_of_open_panel(path));
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
        } else if paths.len() == 1 && !is_drive {
            paths.first().is_some_and(|path| self.can_rename_path(path))
        } else if let Some(path) = drive_target_path {
            path.to_str().is_some_and(|drive_path| {
                crate::infrastructure::windows::drive_supports_volume_label_rename(
                    crate::infrastructure::windows::detect_drive_type(drive_path),
                )
            })
        } else {
            false
        };
        let paste_target = paths
            .first()
            .filter(|path| self.context_target_is_directory(_item_index, path));
        let paste_destination_is_archive = paste_target
            .is_some_and(|path| Self::path_is_archive_namespace(path))
            || (paste_target.is_none() && self.current_location_is_archive_namespace());
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
                    .enabled(can_cut_target),
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
            let can_paste = paste_target
                .map(|path| self.can_paste_into_path(path))
                .unwrap_or_else(|| self.can_paste_into_current_location())
                && !paste_destination_is_archive;
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
                    .enabled(can_delete_target),
            );
        }
        // ========== SECONDARY ITEMS (App-specific) ==========
        let can_create_folder =
            !crate::domain::special_paths::is_virtual_path(&self.navigation_state.current_path)
                && !self.current_location_is_archive_namespace();
        if is_empty_area {
            items.push(ContextMenuItem::separator());
            items.push(grouping_menu_item(self));
            items.push(
                ContextMenuItem::new(-1, t!("context_menu.create_folder"))
                    .with_svg_icon("folder_new")
                    .with_shortcut(
                        self.shortcuts
                            .label(crate::app::shortcuts::ShortcutAction::CreateFolder),
                    )
                    .enabled(can_create_folder),
            );
            if can_create_folder {
                items.push(ContextMenuItem {
                    id: -110,
                    text: t!("context_menu.new").to_string(),
                    is_enabled: false,
                    svg_icon_name: Some("folder_new".to_string()),
                    is_loading_placeholder: true,
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
        } else {
            items.push(ContextMenuItem::separator());
            items.push(ContextMenuItem::new(-20, t!("context_menu.open")).with_svg_icon("folder"));
            if can_play_optical_disc {
                items.push(
                    ContextMenuItem::new(-82, t!("context_menu.play_optical_disc"))
                        .with_command("play_optical_disc")
                        .with_svg_icon("play"),
                );
            }
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
            // Quick Access stores one real folder per entry. Reuse cached item
            // metadata so cloud and network paths never require blocking I/O here.
            if paths.len() == 1 && !is_drive && !target_is_file {
                let target_path = &paths[0];
                if self.context_target_is_directory(_item_index, target_path) {
                    if let Some(target_path) = target_path.to_str() {
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
                let sorted_ids = self.sorted_tag_ids();
                for (idx, &tag_id) in sorted_ids.iter().enumerate() {
                    let Some(tag) = self.tag_definitions.get(&tag_id) else {
                        continue;
                    };
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
                        .with_svg_icon("tag")
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
            let target = if is_empty_area {
                crate::infrastructure::shell_menu_worker::ShellMenuTarget::FolderBackground(
                    paths
                        .first()
                        .cloned()
                        .unwrap_or_else(|| PathBuf::from(&self.navigation_state.current_path)),
                )
            } else {
                crate::infrastructure::shell_menu_worker::ShellMenuTarget::Selection(paths.to_vec())
            };
            let shell_sent = self
                .shell_menu_req_tx
                .try_send(
                    crate::infrastructure::shell_menu_worker::ShellMenuRequest::Extract {
                        request_id: self.shell_menu_request_id,
                        hwnd_isize: hwnd.0 as isize,
                        target,
                    },
                )
                .is_ok();
            self.shell_menu_loading = shell_sent;

            let open_with_sent = if target_is_file && paths.len() == 1 {
                let sent = self
                    .open_with_req_tx
                    .try_send(
                        crate::infrastructure::open_with_worker::OpenWithRequest::Enumerate {
                            request_id: self.shell_menu_request_id,
                            paths: paths.to_vec(),
                        },
                    )
                    .is_ok();
                self.open_with_loading = sent;
                if !sent {
                    if let Some(index) = items.iter().position(|item| {
                        item.command_string.as_deref() == Some("openwith_placeholder")
                    }) {
                        items[index] = ContextMenuItem::new(
                            crate::infrastructure::open_with_worker::OPEN_WITH_DIALOG_ID,
                            t!("context_menu.open_with"),
                        )
                        .with_command(
                            crate::infrastructure::open_with_worker::OPEN_WITH_DIALOG_COMMAND,
                        );
                    }
                }
                sent
            } else {
                let _ = self
                    .open_with_control_tx
                    .try_send(crate::infrastructure::open_with_worker::OpenWithRequest::Cancel);
                self.open_with_loading = false;
                false
            };
            self.context_menu_workers_active = shell_sent || open_with_sent;

            if shell_sent && should_offer_extract_all(paths, is_empty_area, target_is_file) {
                insert_extract_all_pending_item(&mut items);
            }

            // Add a single loading placeholder for "Show more options".
            // All shell items are placed inside this submenu, so only one
            // placeholder is needed and the menu height stays stable.
            if shell_sent {
                items.push(ContextMenuItem::separator());
                items.push(ContextMenuItem {
                    id: -200,
                    text: t!("context_menu.show_more").to_string(),
                    is_enabled: false,
                    is_loading_placeholder: true,
                    ..Default::default()
                });
            }
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
        use crate::infrastructure::windows::native_menu::{is_filtered_shell_text, is_known_verb};

        let uses_direct_open_with = self.context_menu.items.iter().any(|item| {
            matches!(
                item.command_string.as_deref(),
                Some(
                    "openwith_placeholder"
                        | "open_with_menu"
                        | "open_with_dialog"
                        | "open_with_shell_fallback"
                )
            )
        }) && self.context_menu.target_paths.len() == 1;

        fn convert(ui_ctx: &egui::Context, item: &ShellMenuItemData) -> Option<ContextMenuItem> {
            // Filter verbs handled internally
            if let Some(ref verb) = item.command_string {
                if is_known_verb(verb) {
                    return None;
                }
            }
            if is_filtered_shell_text(&item.text) {
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
        self.context_menu.items.retain(|item| {
            !item.is_loading_placeholder
                || (uses_direct_open_with
                    && item.command_string.as_deref() == Some("openwith_placeholder"))
        });
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
        let mut new_submenu_item: Option<ContextMenuItem> = None;
        let mut all_shell_items = Vec::new();

        for raw in &shell_items {
            let raw_text = raw.text.to_lowercase();
            let is_open_with = raw
                .command_string
                .as_deref()
                .is_some_and(|verb| verb.eq_ignore_ascii_case("openas"))
                || raw_text.contains("open with")
                || raw_text.contains("abrir com");
            if let Some(item) = convert(ctx, raw) {
                if item.is_separator {
                    continue;
                }
                let is_new_submenu = self.context_menu.is_empty_area
                    && (item
                        .command_string
                        .as_deref()
                        .is_some_and(|verb| verb.eq_ignore_ascii_case("new"))
                        || matches!(item.text.to_lowercase().as_str(), "new" | "novo"));
                if is_new_submenu {
                    new_submenu_item = Some(item);
                    continue;
                }
                // Promote "Open with" to the main menu only for files
                if target_is_file && is_open_with {
                    open_with_item = Some(item);
                } else {
                    all_shell_items.push(item);
                }
            }
        }

        let mut pending_open_with_submenu_load = None;
        let items = &mut self.context_menu.items;

        // Remove the Open with placeholder before inserting the real item
        if !uses_direct_open_with {
            if let Some(idx) = items
                .iter()
                .position(|i| i.command_string.as_deref() == Some("openwith_placeholder"))
            {
                items.remove(idx);
            }
        }

        // Insert the shell "Open with" right after "Open in new tab" (-21)
        if let Some(mut open_with) = open_with_item {
            // Translate the text to match the current locale
            open_with.text = t!("context_menu.open_with").to_string();
            let needs_submenu_load =
                open_with.has_pending_submenu && open_with.sub_items.is_empty();
            if uses_direct_open_with {
                open_with.command_string = Some("open_with_shell_fallback".to_string());
                if let Some(index) = items.iter().position(|item| {
                    matches!(
                        item.command_string.as_deref(),
                        Some("openwith_placeholder" | "open_with_dialog")
                    )
                }) {
                    if needs_submenu_load {
                        pending_open_with_submenu_load = Some(open_with.id);
                    }
                    items[index] = open_with;
                    // The native submenu is already usable even if direct
                    // association enumeration is still running.
                    self.open_with_loading = false;
                }
            } else if let Some(idx) = items.iter().position(|i| i.id == -21) {
                if needs_submenu_load {
                    pending_open_with_submenu_load = Some(open_with.id);
                }
                items.insert(idx + 1, open_with);
            } else {
                if needs_submenu_load {
                    pending_open_with_submenu_load = Some(open_with.id);
                }
                // Fallback: append before the separator that precedes shell items
                items.push(open_with);
            }
        }

        promote_extract_all_shell_item(items, &mut all_shell_items);

        if let Some(mut new_submenu) = new_submenu_item {
            new_submenu.text = t!("context_menu.new").to_string();
            if new_submenu.icon.is_none() {
                new_submenu.svg_icon_name = Some("folder_new".to_string());
            }
            if let Some(idx) = items.iter().position(|item| item.id == -1) {
                items.insert(idx + 1, new_submenu);
            } else {
                items.push(new_submenu);
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

    pub fn apply_open_with_items(
        &mut self,
        items: Vec<crate::infrastructure::open_with_worker::OpenWithItemData>,
    ) {
        use crate::application::context_menu::ContextMenuItem;
        use crate::infrastructure::open_with_worker::{
            OPEN_WITH_DIALOG_COMMAND, OPEN_WITH_DIALOG_ID, OPEN_WITH_HANDLER_COMMAND_PREFIX,
            OPEN_WITH_MENU_COMMAND, OPEN_WITH_PARENT_ID,
        };

        let mut sub_items: Vec<ContextMenuItem> = items
            .into_iter()
            .filter_map(|item| {
                let menu_id =
                    crate::infrastructure::open_with_worker::menu_id_for_handler(item.handler_id)?;
                Some(
                    ContextMenuItem::new(menu_id, item.name).with_command(format!(
                        "{}{}",
                        OPEN_WITH_HANDLER_COMMAND_PREFIX, item.handler_id
                    )),
                )
            })
            .collect();
        if !sub_items.is_empty() {
            sub_items.push(ContextMenuItem::separator());
        }
        sub_items.push(
            ContextMenuItem::new(OPEN_WITH_DIALOG_ID, t!("context_menu.choose_another_app"))
                .with_command(OPEN_WITH_DIALOG_COMMAND),
        );

        let open_with = ContextMenuItem::new(OPEN_WITH_PARENT_ID, t!("context_menu.open_with"))
            .with_command(OPEN_WITH_MENU_COMMAND)
            .with_subitems(sub_items);
        self.replace_open_with_placeholder(open_with);
        self.open_with_loading = false;
    }

    pub fn apply_open_with_fallback(&mut self) {
        use crate::application::context_menu::ContextMenuItem;
        use crate::infrastructure::open_with_worker::{
            OPEN_WITH_DIALOG_COMMAND, OPEN_WITH_DIALOG_ID,
        };

        if self
            .context_menu
            .items
            .iter()
            .any(|item| item.command_string.as_deref() == Some("open_with_shell_fallback"))
        {
            self.open_with_loading = false;
            return;
        }

        let fallback = ContextMenuItem::new(OPEN_WITH_DIALOG_ID, t!("context_menu.open_with"))
            .with_command(OPEN_WITH_DIALOG_COMMAND)
            .with_svg_icon("external-link");
        self.replace_open_with_placeholder(fallback);
        self.open_with_loading = false;
    }

    pub fn apply_open_with_icon(
        &mut self,
        handler_id: u32,
        rgba: Vec<u8>,
        width: u32,
        height: u32,
        ctx: &egui::Context,
    ) {
        let Some(item_id) =
            crate::infrastructure::open_with_worker::menu_id_for_handler(handler_id)
        else {
            return;
        };
        let Some(parent) =
            self.context_menu.items.iter_mut().find(|item| {
                item.id == crate::infrastructure::open_with_worker::OPEN_WITH_PARENT_ID
            })
        else {
            return;
        };
        let Some(item) = parent.sub_items.iter_mut().find(|item| item.id == item_id) else {
            return;
        };
        item.icon = Some(ctx.load_texture(
            format!(
                "open_with_icon_{}_{}",
                self.shell_menu_request_id, handler_id
            ),
            egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &rgba),
            Default::default(),
        ));
        item.svg_icon_name = None;
    }

    fn replace_open_with_placeholder(
        &mut self,
        item: crate::application::context_menu::ContextMenuItem,
    ) {
        if let Some(index) = self.context_menu.items.iter().position(|existing| {
            matches!(
                existing.command_string.as_deref(),
                Some(
                    "openwith_placeholder"
                        | "open_with_menu"
                        | "open_with_dialog"
                        | "open_with_shell_fallback"
                )
            )
        }) {
            let replaced_shell_id = (self.context_menu.items[index].command_string.as_deref()
                == Some("open_with_shell_fallback"))
            .then_some(self.context_menu.items[index].id);
            self.context_menu.items[index] = item;
            if let Some(shell_id) = replaced_shell_id {
                if self.context_menu.pending_load_item == Some(shell_id) {
                    self.context_menu.pending_load_item = None;
                }
                self.context_menu.finish_submenu_load(shell_id);
            }
        } else if let Some(index) = self
            .context_menu
            .items
            .iter()
            .position(|item| item.id == -21)
        {
            self.context_menu.items.insert(index + 1, item);
        }
        self.context_menu.partition_items();
    }

    pub fn handle_lazy_submenu_load(&mut self, _egui_ctx: &egui::Context, item_id: i32) {
        if !self.context_menu.begin_submenu_load(item_id) {
            return;
        }

        // The ShellMenuContext now lives exclusively on the worker thread.
        // Send a LoadSubmenu request; the SubmenuLoaded response is processed in
        // the update-loop polling code which calls `apply_async_submenu_items`.
        if self
            .shell_menu_req_tx
            .try_send(
                crate::infrastructure::shell_menu_worker::ShellMenuRequest::LoadSubmenu {
                    request_id: self.shell_menu_request_id,
                    item_id: item_id as u32,
                },
            )
            .is_err()
        {
            self.context_menu.finish_submenu_load(item_id);
            self.shell_menu_loading = !self.context_menu.loading_submenu_ids.is_empty();
            return;
        }
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

        fn is_new_submenu(items: &[ContextMenuItem], id: i32) -> bool {
            items.iter().any(|item| {
                (item.id == id
                    && item
                        .command_string
                        .as_deref()
                        .is_some_and(|verb| verb.eq_ignore_ascii_case("new")))
                    || is_new_submenu(&item.sub_items, id)
            })
        }

        let hide_native_folder = is_new_submenu(&self.context_menu.items, item_id as i32);
        self.context_menu.finish_submenu_load(item_id as i32);
        let new_subitems: Vec<ContextMenuItem> = sub_items
            .iter()
            .filter(|item| {
                !hide_native_folder
                    || !item
                        .command_string
                        .as_deref()
                        .is_some_and(|verb| verb.eq_ignore_ascii_case("newfolder"))
            })
            .map(|item| convert_item(ctx, item))
            .collect();
        update_ui_item(&mut self.context_menu.items, item_id as i32, new_subitems);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        extract_all_shell_command_id, insert_extract_all_pending_item, is_extract_all_pending_item,
        is_extract_all_shell_item, is_optical_disc_context_target, promote_extract_all_shell_item,
        should_offer_extract_all,
    };
    use crate::application::context_menu::{
        ContextMenuItem, EXTRACT_ALL_PENDING_COMMAND, EXTRACT_ALL_PENDING_ID,
    };
    use crate::infrastructure::windows::DriveType;
    use std::path::PathBuf;

    #[test]
    fn recognizes_extract_all_shell_command() {
        let by_verb = ContextMenuItem::new(10, "Localized text").with_command("ExtractAll");
        let by_english_text = ContextMenuItem::new(11, "Extract All...");
        let by_portuguese_text = ContextMenuItem::new(12, "Extrair Tudo\u{2026}");
        let different_command =
            ContextMenuItem::new(13, "Extract here").with_command("extracthere");

        assert!(is_extract_all_shell_item(&by_verb));
        assert!(is_extract_all_shell_item(&by_english_text));
        assert!(is_extract_all_shell_item(&by_portuguese_text));
        assert!(!is_extract_all_shell_item(&different_command));
    }

    #[test]
    fn promotes_extract_all_directly_after_open_with() {
        for open_with_command in [
            "openwith_placeholder",
            "open_with_menu",
            "open_with_dialog",
            "open_with_shell_fallback",
            "OpenAs",
        ] {
            let mut menu_items = vec![
                ContextMenuItem::new(-20, "Open"),
                ContextMenuItem::new(-201, "Open with").with_command(open_with_command),
                ContextMenuItem::new(-80, "Open in terminal"),
            ];
            let mut shell_items = vec![
                ContextMenuItem::new(10, "Share").with_command("share"),
                ContextMenuItem::new(11, "Extract All").with_command("extractall"),
            ];

            promote_extract_all_shell_item(&mut menu_items, &mut shell_items);

            assert_eq!(
                menu_items[1].command_string.as_deref(),
                Some(open_with_command)
            );
            assert_eq!(menu_items[2].command_string.as_deref(), Some("extractall"));
            assert_eq!(menu_items[2].svg_icon_name.as_deref(), Some("extract_all"));
            assert_eq!(menu_items[3].id, -80);
            assert_eq!(shell_items.len(), 1);
            assert!(!shell_items.iter().any(is_extract_all_shell_item));
        }
    }

    #[test]
    fn inserts_and_replaces_extract_all_pending_item_in_place() {
        let mut menu_items = vec![
            ContextMenuItem::new(-201, "Open with").with_command("open_with_menu"),
            ContextMenuItem::new(-80, "Open in terminal"),
        ];

        insert_extract_all_pending_item(&mut menu_items);

        assert_eq!(menu_items[1].id, EXTRACT_ALL_PENDING_ID);
        assert_eq!(
            menu_items[1].command_string.as_deref(),
            Some(EXTRACT_ALL_PENDING_COMMAND)
        );
        assert_eq!(menu_items[1].svg_icon_name.as_deref(), Some("extract_all"));

        let mut shell_items = vec![
            ContextMenuItem::new(10, "Share").with_command("share"),
            ContextMenuItem::new(11, "Extract All").with_command("extractall"),
        ];
        promote_extract_all_shell_item(&mut menu_items, &mut shell_items);

        assert_eq!(menu_items[1].id, 11);
        assert_eq!(menu_items[1].command_string.as_deref(), Some("extractall"));
        assert_eq!(menu_items[1].svg_icon_name.as_deref(), Some("extract_all"));
        assert_eq!(menu_items[2].id, -80);
        assert!(!shell_items.iter().any(is_extract_all_shell_item));
    }

    #[test]
    fn removes_pending_extract_all_when_shell_does_not_offer_it() {
        let mut menu_items = vec![
            ContextMenuItem::new(-201, "Open with").with_command("open_with_menu"),
            ContextMenuItem::new(EXTRACT_ALL_PENDING_ID, "Extract All")
                .with_command(EXTRACT_ALL_PENDING_COMMAND),
        ];
        let mut shell_items = vec![ContextMenuItem::new(10, "Share").with_command("share")];

        promote_extract_all_shell_item(&mut menu_items, &mut shell_items);

        assert!(!menu_items.iter().any(is_extract_all_pending_item));
        assert_eq!(shell_items.len(), 1);
    }

    #[test]
    fn resolves_only_enabled_native_extract_all_commands() {
        let menu_items = vec![
            ContextMenuItem::new(EXTRACT_ALL_PENDING_ID, "Extract All")
                .with_command(EXTRACT_ALL_PENDING_COMMAND),
            ContextMenuItem::new(10, "Extract All")
                .with_command("extractall")
                .enabled(false),
            ContextMenuItem::new(11, "Extract All").with_command("extractall"),
        ];

        assert_eq!(extract_all_shell_command_id(&menu_items), Some(11));
    }

    #[test]
    fn offers_extract_all_immediately_only_for_one_archive_file() {
        assert!(should_offer_extract_all(
            &[PathBuf::from(r"C:\files\archive.zip")],
            false,
            true
        ));
        assert!(!should_offer_extract_all(
            &[PathBuf::from(r"C:\files\document.txt")],
            false,
            true
        ));
        assert!(!should_offer_extract_all(
            &[
                PathBuf::from(r"C:\files\one.zip"),
                PathBuf::from(r"C:\files\two.zip")
            ],
            false,
            true
        ));
        assert!(!should_offer_extract_all(
            &[PathBuf::from(r"C:\files\archive.zip")],
            true,
            true
        ));
    }

    #[test]
    fn keeps_extract_all_in_shell_items_without_open_with_anchor() {
        let mut menu_items = vec![ContextMenuItem::new(-20, "Open")];
        let mut shell_items =
            vec![ContextMenuItem::new(11, "Extract All").with_command("extractall")];

        promote_extract_all_shell_item(&mut menu_items, &mut shell_items);

        assert_eq!(menu_items.len(), 1);
        assert!(shell_items.iter().any(is_extract_all_shell_item));
    }

    #[test]
    fn offers_playback_only_for_one_optical_drive_root() {
        assert!(is_optical_disc_context_target(
            false,
            1,
            true,
            Some(DriveType::Cdrom)
        ));
        assert!(!is_optical_disc_context_target(
            false,
            2,
            true,
            Some(DriveType::Cdrom)
        ));
        assert!(!is_optical_disc_context_target(
            true,
            1,
            true,
            Some(DriveType::Cdrom)
        ));
        assert!(!is_optical_disc_context_target(
            false,
            1,
            false,
            Some(DriveType::Cdrom)
        ));
        assert!(!is_optical_disc_context_target(
            false,
            1,
            true,
            Some(DriveType::Fixed)
        ));
    }
}
