//! Context menu population
//!
//! This module handles population of the right-click context menu, merging native Shell items.

use crate::app::state::ImageViewerApp;
use eframe::egui;
use std::path::PathBuf;

impl ImageViewerApp {
    pub fn context_target_paths(&self, item_idx: Option<usize>) -> Vec<PathBuf> {
        // 1. Prioritize context menu state (populated by right-click)
        if !self.context_menu.target_paths.is_empty() {
            return self.context_menu.target_paths.clone();
        }

        // 2. Explicit item index
        if let Some(idx) = item_idx {
            if let Some(i) = self.items.get(idx) {
                return vec![i.path.clone()];
            }
        }

        // 3. Multi-selection
        if !self.multi_selection.is_empty() {
            return self.multi_selection.iter().cloned().collect();
        }

        // 4. Single selection
        if let Some(sel) = &self.selected_file {
            return vec![sel.path.clone()];
        }

        // 5. Current folder
        vec![PathBuf::from(&self.navigation_state.current_path)]
    }

    pub fn populate_context_menu(
        &mut self,
        _ctx: &egui::Context,
        paths: &[PathBuf],
        is_empty_area: bool,
        _item_index: Option<usize>,
    ) {
        use crate::application::context_menu::ContextMenuItem;

        let mut items = Vec::new();

        // Special menu for Recycle Bin items
        if self.navigation_state.is_recycle_bin_view && !is_empty_area {
            // Menu items for recycle bin (no primary icons)
            items.push(ContextMenuItem::new(-52, "Restaurar").with_command("restore"));
            items.push(
                ContextMenuItem::new(-53, "Excluir permanentemente")
                    .with_command("delete_permanent"),
            );
            items.push(ContextMenuItem::separator());
            items.push(
                ContextMenuItem::new(-28, "Propriedades")
                    .with_command("properties")
                    .with_shortcut("Alt+Enter"),
            );

            self.context_menu.items = items;
            return;
        }

        // Special menu for empty area in Recycle Bin
        if self.navigation_state.is_recycle_bin_view && is_empty_area {
            items.push(
                ContextMenuItem::new(-54, "Esvaziar Lixeira").with_command("empty_recycle_bin"),
            );
            self.context_menu.items = items;
            return;
        }

        // Check if the target item is a drive (drives don't support file operations)
        let is_drive = _item_index
            .and_then(|idx| self.items.get(idx))
            .map(|item| item.drive_info.is_some())
            .unwrap_or(false);

        // ========== PRIMARY ITEMS (Header bar) - matching Files ==========
        // These appear as icon buttons in the header
        items.push(
            ContextMenuItem::primary(-3, "Recortar")
                .with_command("cut")
                .with_shortcut("Ctrl+X")
                .enabled(!is_drive),
        );
        items.push(
            ContextMenuItem::primary(-2, "Copiar")
                .with_command("copy")
                .with_shortcut("Ctrl+C")
                .enabled(!is_drive),
        );

        let can_paste = self.clipboard.has_content();
        items.push(
            ContextMenuItem::primary(-4, "Colar")
                .with_command("paste")
                .with_shortcut("Ctrl+V")
                .enabled(can_paste && !is_drive),
        );

        if !is_empty_area {
            items.push(
                ContextMenuItem::primary(-5, "Renomear")
                    .with_command("rename")
                    .with_shortcut("F2"),
            );
            items.push(
                ContextMenuItem::primary(-6, "Excluir")
                    .with_command("delete")
                    .with_shortcut("Del")
                    .enabled(!is_drive),
            );
        }

        // ========== SECONDARY ITEMS (App-specific) ==========
        let can_paste = self.clipboard.has_content();
        let can_create_folder =
            !self.navigation_state.is_computer_view && !self.navigation_state.is_recycle_bin_view;
        if is_empty_area {
            items.push(ContextMenuItem::separator());
            items.push(
                ContextMenuItem::new(-32, "Colar")
                    .with_command("paste")
                    .with_shortcut("Ctrl+V")
                    .enabled(can_paste),
            );
            items.push(
                ContextMenuItem::new(-1, "Criar pasta")
                    .with_shortcut("Ctrl+Shift+N")
                    .enabled(can_create_folder),
            );
        } else {
            items.push(ContextMenuItem::separator());
            items.push(ContextMenuItem::new(-20, "Abrir"));
            items.push(ContextMenuItem::new(-21, "Abrir em nova guia"));
            items.push(ContextMenuItem::separator());
            // Basic file operations as text items (in addition to header icons)
            items.push(
                ContextMenuItem::new(-30, "Recortar")
                    .with_command("cut")
                    .with_shortcut("Ctrl+X")
                    .enabled(!is_drive),
            );
            items.push(
                ContextMenuItem::new(-31, "Copiar")
                    .with_command("copy")
                    .with_shortcut("Ctrl+C")
                    .enabled(!is_drive),
            );
            items.push(
                ContextMenuItem::new(-32, "Colar")
                    .with_command("paste")
                    .with_shortcut("Ctrl+V")
                    .enabled(can_paste && !is_drive),
            );
            items.push(
                ContextMenuItem::new(-33, "Renomear")
                    .with_command("rename")
                    .with_shortcut("F2"),
            );
            items.push(
                ContextMenuItem::new(-34, "Excluir")
                    .with_command("delete")
                    .with_shortcut("Del")
                    .enabled(!is_drive),
            );
            items.push(ContextMenuItem::separator());
            items.push(ContextMenuItem::new(-24, "Copiar caminho").with_shortcut("Ctrl+Shift+C"));
            items.push(ContextMenuItem::new(-26, "Criar atalho"));
            // Quick Access pin/unpin — only for folders (not drives)
            if !is_drive {
                if let Some(target_path) = paths.first().and_then(|p| p.to_str()) {
                    // Use cached is_dir field — avoids blocking I/O on OneDrive/network paths
                    let target_is_dir = _item_index
                        .and_then(|idx| self.items.get(idx))
                        .map(|item| item.is_dir)
                        .unwrap_or_else(|| {
                            // Fallback: search already-loaded items by path (no I/O)
                            self.items
                                .iter()
                                .find(|it| it.path.to_str() == Some(target_path))
                                .map(|it| it.is_dir)
                                .unwrap_or(false)
                        });
                    if target_is_dir {
                        let is_pinned = self
                            .pinned_folders
                            .iter()
                            .any(|pf| pf.path == target_path);
                        items.push(ContextMenuItem::separator());
                        if is_pinned {
                            items.push(ContextMenuItem::new(-61, "Remover do Acesso Rápido"));
                        } else {
                            items.push(ContextMenuItem::new(-60, "Fixar no Acesso Rápido"));
                        }
                    }
                }
            }

            items.push(ContextMenuItem::separator());
            items.push(
                ContextMenuItem::new(-28, "Propriedades")
                    .with_command("properties")
                    .with_shortcut("Alt+Enter"),
            );
        }

        // ========== SHELL ITEMS — extracted asynchronously on the worker thread ==========
        // Dispatch to the STA worker so Shell extensions cannot block the UI thread.
        // Results arrive via `shell_menu_res_rx`; the app polls them in its update loop
        // and calls `apply_async_shell_items` to merge them into `self.context_menu.items`.
        if let Some(hwnd) = self.native_hwnd {
            let _ = self.shell_menu_req_tx.send(
                crate::infrastructure::shell_menu_worker::ShellMenuRequest::Extract {
                    hwnd_isize: hwnd.0 as isize,
                    paths: paths.to_vec(),
                },
            );
            self.shell_menu_loading = true;
        }

        self.context_menu.items = items;
    }

    /// Convert `ShellMenuItemData` items received from the worker and merge them into
    /// the already-populated context menu.  Called from the update-loop polling code.
    pub fn apply_async_shell_items(
        &mut self,
        shell_items: Vec<crate::infrastructure::shell_menu_worker::ShellMenuItemData>,
        ctx: &egui::Context,
    ) {
        use crate::application::context_menu::ContextMenuItem;
        use crate::infrastructure::windows::native_menu::is_known_verb;
        use crate::infrastructure::shell_menu_worker::ShellMenuItemData;

        fn convert(
            ui_ctx: &egui::Context,
            item: &ShellMenuItemData,
        ) -> Option<ContextMenuItem> {
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

            let sub_items = item.sub_items.iter().filter_map(|s| convert(ui_ctx, s)).collect();

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
            })
        }

        let mut visible = Vec::new();
        let mut overflow = Vec::new();

        for raw in &shell_items {
            if let Some(item) = convert(ctx, raw) {
                if !item.sub_items.is_empty() || item.has_pending_submenu {
                    visible.push(item);
                } else if !item.is_separator {
                    overflow.push(item);
                }
            }
        }

        let items = &mut self.context_menu.items;

        if !visible.is_empty() {
            items.push(ContextMenuItem::separator());
            items.extend(visible);
        }
        if !overflow.is_empty() {
            items.push(ContextMenuItem::separator());
            items.push(ContextMenuItem::new(-99, "Mostrar mais opções").with_subitems(overflow));
        }

        self.shell_menu_loading = false;
    }

    pub fn handle_lazy_submenu_load(&mut self, _egui_ctx: &egui::Context, item_id: i32) {
        // The ShellMenuContext now lives exclusively on the worker thread.
        // Send a LoadSubmenu request; the SubmenuLoaded response is processed in
        // the update-loop polling code which calls `apply_async_submenu_items`.
        let _ = self.shell_menu_req_tx.send(
            crate::infrastructure::shell_menu_worker::ShellMenuRequest::LoadSubmenu {
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
                sub_items: item.sub_items.iter().map(|s| convert_item(ui_ctx, s)).collect(),
                is_separator: item.is_separator,
                is_enabled: item.is_enabled,
                is_primary: false,
                keyboard_shortcut: None,
                command_string: item.command_string.clone(),
                show_in_overflow: false,
                has_pending_submenu: item.has_submenu,
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
