//! Context menu population
//!
//! This module handles population of the right-click context menu, merging native Shell items.

use std::path::PathBuf;
use eframe::egui;
use crate::app::state::ImageViewerApp;

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
        vec![PathBuf::from(&self.current_path)]
    }

    pub fn populate_context_menu(
        &mut self,
        ctx: &egui::Context,
        paths: &[PathBuf],
        is_empty_area: bool,
        _item_index: Option<usize>,
    ) {
        use crate::application::context_menu::ContextMenuItem;
        use crate::infrastructure::windows::native_menu::{
            extract_shell_menu, is_known_verb, ShellMenuItem,
        };

        let mut items = Vec::new();

        // Special menu for Recycle Bin items
        if self.is_recycle_bin_view && !is_empty_area {
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
        if self.is_recycle_bin_view && is_empty_area {
            items.push(
                ContextMenuItem::new(-54, "Esvaziar Lixeira").with_command("empty_recycle_bin"),
            );
            self.context_menu.items = items;
            return;
        }

        // ========== PRIMARY ITEMS (Header bar) - matching Files ==========
        // These appear as icon buttons in the header
        items.push(
            ContextMenuItem::primary(-3, "Recortar")
                .with_command("cut")
                .with_shortcut("Ctrl+X"),
        );
        items.push(
            ContextMenuItem::primary(-2, "Copiar")
                .with_command("copy")
                .with_shortcut("Ctrl+C"),
        );

        let can_paste = self.clipboard.has_content();
        items.push(
            ContextMenuItem::primary(-4, "Colar")
                .with_command("paste")
                .with_shortcut("Ctrl+V")
                .enabled(can_paste),
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
                    .with_shortcut("Del"),
            );
        }

        // ========== SECONDARY ITEMS (App-specific) ==========
        let can_paste = self.clipboard.has_content();
        let can_create_folder = !self.is_computer_view && !self.is_recycle_bin_view;
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
                    .with_shortcut("Ctrl+X"),
            );
            items.push(
                ContextMenuItem::new(-31, "Copiar")
                    .with_command("copy")
                    .with_shortcut("Ctrl+C"),
            );
            items.push(
                ContextMenuItem::new(-32, "Colar")
                    .with_command("paste")
                    .with_shortcut("Ctrl+V")
                    .enabled(can_paste),
            );
            items.push(
                ContextMenuItem::new(-33, "Renomear")
                    .with_command("rename")
                    .with_shortcut("F2"),
            );
            items.push(
                ContextMenuItem::new(-34, "Excluir")
                    .with_command("delete")
                    .with_shortcut("Del"),
            );
            items.push(ContextMenuItem::separator());
            items.push(ContextMenuItem::new(-24, "Copiar caminho").with_shortcut("Ctrl+Shift+C"));
            items.push(ContextMenuItem::new(-26, "Criar atalho"));
            items.push(ContextMenuItem::separator());
            items.push(
                ContextMenuItem::new(-28, "Propriedades")
                    .with_command("properties")
                    .with_shortcut("Alt+Enter"),
            );
        }

        // ========== SHELL ITEMS (Third-party extensions) ==========
        if let Some(hwnd) = self.native_hwnd {
            if let Ok(shell_ctx) = extract_shell_menu(hwnd, paths) {
                // Convert Shell items to UI items, filtering known verbs
                fn convert(
                    ui_ctx: &egui::Context,
                    shell_item: &ShellMenuItem,
                ) -> Option<ContextMenuItem> {
                    // Filter items we handle internally
                    if let Some(ref verb) = shell_item.command_string {
                        if is_known_verb(verb) {
                            return None;
                        }
                    }

                    // Fallback text-based filter for localized or verbless items
                    let lower_text = shell_item.text.to_lowercase();
                    let blacklisted_texts = [
                        "pin to quick access",
                        "fixar no acesso rápido",
                        "restore previous versions",
                        "restaurar versões anteriores",
                        "copy as path",
                        "copiar como caminho",
                        "create shortcut",
                        "criar atalho",
                    ];
                    if blacklisted_texts.iter().any(|&t| lower_text.contains(t)) {
                        return None;
                    }

                    // Resize icon to 16x16 if needed
                    let icon = shell_item.icon_rgba.as_ref().map(|(rgba, w, h)| {
                        let (final_rgba, fw, fh) = if *w != 16 || *h != 16 {
                            // Simple resize - in production would use proper resampling
                            (rgba.clone(), *w, *h)
                        } else {
                            (rgba.clone(), *w, *h)
                        };
                        let color_image = egui::ColorImage::from_rgba_unmultiplied(
                            [fw as usize, fh as usize],
                            &final_rgba,
                        );
                        ui_ctx.load_texture(
                            format!("menu_icon_{}", shell_item.id),
                            color_image,
                            Default::default(),
                        )
                    });

                    let sub_items: Vec<ContextMenuItem> = shell_item
                        .sub_items
                        .iter()
                        .filter_map(|s| convert(ui_ctx, s))
                        .collect();

                    Some(ContextMenuItem {
                        id: shell_item.id as i32,
                        text: shell_item.text.clone(),
                        icon,
                        sub_items,
                        is_separator: shell_item.is_separator,
                        is_enabled: shell_item.is_enabled,
                        is_primary: false,
                        keyboard_shortcut: None,
                        command_string: shell_item.command_string.clone(),
                        show_in_overflow: false,
                        has_pending_submenu: shell_item.pending_submenu_handle.is_some(),
                    })
                }

                let shell_items: Vec<ContextMenuItem> = shell_ctx
                    .items
                    .borrow()
                    .iter()
                    .filter_map(|s| convert(ctx, s))
                    .collect();

                // Separate shell items: common ones visible, rest go to overflow
                let mut visible_shell_items = Vec::new();
                let mut overflow_shell_items = Vec::new();

                for s_item in shell_items {
                    // Keep items with submenus OR pending submenus (like 7-Zip, WinRAR) visible
                    if !s_item.sub_items.is_empty() || s_item.has_pending_submenu {
                        visible_shell_items.push(s_item);
                    } else if !s_item.is_separator {
                        overflow_shell_items.push(s_item);
                    }
                }

                // Add visible shell items (with submenus like 7-Zip)
                if !visible_shell_items.is_empty() {
                    items.push(ContextMenuItem::separator());
                    for s_item in visible_shell_items {
                        items.push(s_item);
                    }
                }

                // Add overflow submenu with remaining shell items
                if !overflow_shell_items.is_empty() {
                    items.push(ContextMenuItem::separator());
                    items.push(
                        ContextMenuItem::new(-99, "Mostrar mais opções")
                            .with_subitems(overflow_shell_items),
                    );
                }

                // Keep the native context alive for command invocation
                self.context_menu.native_context = Some(std::rc::Rc::new(shell_ctx));
            }
        }

        self.context_menu.items = items;
    }

    pub fn handle_lazy_submenu_load(&mut self, egui_ctx: &egui::Context, item_id: i32) {
        use crate::infrastructure::windows::native_menu::{ShellMenuContext, ShellMenuItem};
        use crate::application::context_menu::ContextMenuItem;

        let native_ctx = self.context_menu.native_context.clone();
        let Some(native_ctx) = native_ctx else { return };
        // Use as_ref() to get &dyn Any before downcasting
        let Some(shell_ctx) = native_ctx.as_ref().downcast_ref::<ShellMenuContext>() else { return };

        // 1. Find the ShellMenuItem recursively
        fn find_shell_item_mut<'a>(items: &'a mut [ShellMenuItem], id: u32) -> Option<&'a mut ShellMenuItem> {
            for item in items {
                if item.id == id {
                    return Some(item);
                }
                if let Some(found) = find_shell_item_mut(&mut item.sub_items, id) {
                    return Some(found);
                }
            }
            None
        }

        let mut items = shell_ctx.items.borrow_mut();
        if let Some(shell_item) = find_shell_item_mut(&mut items, item_id as u32) {
            if shell_ctx.load_pending_submenu(shell_item) {
                // 2. Success! Now update the ContextMenuItem tree
                fn convert_item(ui_ctx: &egui::Context, shell_item: &ShellMenuItem) -> ContextMenuItem {
                    let icon = shell_item.icon_rgba.as_ref().map(|(rgba, w, h)| {
                        ui_ctx.load_texture(
                            format!("menu_icon_{}", shell_item.id),
                            egui::ColorImage::from_rgba_unmultiplied([*w as usize, *h as usize], rgba),
                            Default::default(),
                        )
                    });

                    ContextMenuItem {
                        id: shell_item.id as i32,
                        text: shell_item.text.clone(),
                        icon,
                        sub_items: shell_item.sub_items.iter().map(|s| convert_item(ui_ctx, s)).collect(),
                        is_separator: shell_item.is_separator,
                        is_enabled: shell_item.is_enabled,
                        is_primary: false,
                        keyboard_shortcut: None,
                        command_string: shell_item.command_string.clone(),
                        show_in_overflow: false,
                        has_pending_submenu: shell_item.pending_submenu_handle.is_some(),
                    }
                }

                fn update_ui_item(items: &mut [ContextMenuItem], id: i32, new_subitems: Vec<ContextMenuItem>) -> bool {
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

                let new_subitems: Vec<ContextMenuItem> = shell_item.sub_items.iter().map(|s| convert_item(egui_ctx, s)).collect();
                update_ui_item(&mut self.context_menu.items, item_id, new_subitems);
            }
        }
    }
}
