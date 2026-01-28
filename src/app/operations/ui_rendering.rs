//! UI Rendering functions: list_view, grid_view, item_slot
//!
//! This module contains the main logic for rendering the file lists in different modes.
//! It bridges the App state with the UI components.

use eframe::egui;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::app::state::ImageViewerApp;
use crate::ui::components::item_slot::{render_item_slot, ItemSlotContext};
use crate::ui::views::{grid_view, GridViewContext, GridViewOperations};
use crate::ui::views::{list_view, ListViewContext, ListViewOperations};

// Helper function equivalent to open_with_shell from ops
fn open_with_shell(path: &Path) {
    let _ = crate::application::file_operations::open_with_shell(path, None);
}

impl ImageViewerApp {
    // --- DETALHES (LIST VIEW) ---
    pub fn render_list_view(&mut self, ui: &mut egui::Ui) {
        // Keyboard navigation for list view (ONLY when not renaming and media is NOT focused)
        if self.renaming_state.is_none() && !self.is_media_keyboard_focused() {
            let current_index = self.items.iter().position(|x| {
                self.selected_file
                    .as_ref()
                    .map_or(false, |f| f.path == x.path)
            });

            let mut pending_delta: i32 = 0;
            let mut page_action: Option<bool> = None; // true = PageDown, false = PageUp

            if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
                pending_delta += 1;
            }
            if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
                pending_delta -= 1;
            }
            if ui.input(|i| i.key_pressed(egui::Key::PageDown)) {
                page_action = Some(true);
            }
            if ui.input(|i| i.key_pressed(egui::Key::PageUp)) {
                page_action = Some(false);
            }

            let shift = ui.input(|i| i.modifiers.shift);
            let row_height = 24.0;
            let header_h = 32.0; // Header + Separator precise height for visibility
            let viewport_h = (ui.available_height() - header_h).max(0.0);
            let visible_count = (viewport_h / row_height).floor() as usize;

            let mut new_index = None;
            if let Some(is_down) = page_action {
                if is_down {
                    new_index = Some(
                        current_index
                            .map(|idx| {
                                (idx + visible_count).min(self.items.len().saturating_sub(1))
                            })
                            .unwrap_or(visible_count),
                    );
                } else {
                    // PageUp based on viewport, not selected_index
                    let first_visible_index = (self.scroll_offset_y / row_height).floor() as usize;
                    new_index = Some(first_visible_index.saturating_sub(visible_count));
                }
            } else if pending_delta != 0 {
                new_index = Some(
                    current_index
                        .map(|idx| {
                            (idx as i32 + pending_delta)
                                .clamp(0, self.items.len().saturating_sub(1) as i32)
                                as usize
                        })
                        .unwrap_or(0),
                );
            }

            if let Some(idx) = new_index {
                let clamped = idx.min(self.items.len().saturating_sub(1));
                if let Some(item) = self.items.get(clamped) {
                    let item_path = item.path.clone();
                    let is_dir = item.is_dir;

                    // UPDATED: Decoupled Focus (selected_item) from Selection (multi_selection)
                    let old_focus = self.selected_item;
                    self.selected_item = Some(clamped);
                    self.selected_file = Some(item.clone());
                    self.update_selected_thumbnail();
                    self.last_keyboard_nav = Instant::now();

                    if shift {
                        // Shift + Arrow/Page: Range selection
                        if self.selection_anchor.is_none() {
                            self.selection_anchor = old_focus;
                        }
                        if let Some(anchor) = self.selection_anchor {
                            let (start, end) = if anchor < clamped {
                                (anchor, clamped)
                            } else {
                                (clamped, anchor)
                            };
                            // Range between anchor and focus (Add-only as per "NÃO limpar seleção fora do range")
                            for i in start..=end {
                                if let Some(it) = self.items.get(i) {
                                    self.multi_selection.insert(it.path.clone());
                                }
                            }
                        }
                    } else {
                        // Navigation without shift: Single-item selection (clear + add focused item)
                        // This ensures the focused item shows the dark blue selection border
                        self.multi_selection.clear();
                        self.multi_selection.insert(item_path.clone());
                        self.selection_anchor = Some(clamped);
                    }

                    // Trigger scroll normalization in the view
                    self.scroll_to_selected = true;

                    // Request visibility for the new selected index
                    self.scroll_request = crate::app::state::ScrollRequest::EnsureVisible(clamped);

                    ui.ctx().request_repaint();

                    if !is_dir {
                        if !self.cache_manager.has_thumbnail(&item_path)
                            && !self.cache_manager.is_loading(&item_path)
                        {
                            self.request_thumbnail_load(item_path, 512);
                        }
                    }
                }
            }

            // Enter to open (only when not renaming)
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                if let Some(selected) = &self.selected_file.clone() {
                    if selected.is_dir {
                        self.navigate_to(&selected.path.to_string_lossy());
                        return; // Exit early after navigation
                    } else {
                        open_with_shell(&selected.path);
                    }
                }
            }
        }

        // Extrair dados necessários para evitar múltiplos borrows
        let items = self.items.clone(); // Clone para evitar borrow
        let selected_item = self.selected_item;
        let selected_file = self.selected_file.clone();
        let sort_mode = self.sort_mode;
        let sort_descending = self.sort_descending;
        let renaming_state = self.renaming_state.clone();
        let focus_rename = self.focus_rename;
        let folder_icon_texture = self.cache_manager.folder_icon_texture.clone();
        let computer_icon = self.cache_manager.computer_icon.clone();

        // Check if current path is in OneDrive
        let is_onedrive_folder =
            crate::infrastructure::onedrive::is_onedrive_path(&PathBuf::from(&self.current_path));

        // Criar contexto com referências mutáveis separadas
        let scroll_to_selected = self.scroll_to_selected;
        let is_video_playing_docked = self.is_video_playing_docked();
        let multi_selection = &self.multi_selection;
        let mut ctx = ListViewContext {
            items: &items,
            selected_item,
            selected_file: selected_file.as_ref(),
            multi_selection,
            sort_mode,
            sort_descending,
            renaming_state: renaming_state.clone(),
            focus_rename,
            scroll_to_selected,
            is_computer_view: self.is_computer_view,
            is_recycle_bin_view: self.is_recycle_bin_view,
            is_onedrive_folder,
            texture_cache: &mut self.cache_manager.texture_cache,
            loading_set: &mut self.cache_manager.loading_set,
            loading_icons: &mut self.loading_icons,
            failed_icons: &self.failed_icons,
            scanned_folders: &mut self.scanned_folders,
            folder_icon_texture: folder_icon_texture.as_ref(),
            computer_icon: computer_icon.as_ref(),
            drive_icon_cache: &mut self.cache_manager.drive_icon_cache,
            item_icon_loader: &mut self.item_icon_loader,
            deletion_date_cache: Some(&mut self.deletion_date_cache),
            failed_thumbnails: &self.cache_manager.failed_thumbnails,
            scroll_offset_y: self.scroll_offset_y,
            mut_scroll_offset_y: &mut self.scroll_offset_y,
            last_input: self.last_input,
            last_scroll_time: &mut self.last_scroll_time,
            last_scroll_offset: &mut self.last_scroll_offset,
            pending_upload_set: &mut self.cache_manager.pending_upload_set,
            is_video_playing_docked,
        };

        // Usar uma abordagem diferente: coletar ações em vetores
        let mut actions = Vec::new();

        struct ListOps<'a> {
            actions: &'a mut Vec<ListAction>,
        }

        enum ListAction {
            NavigateTo(String),
            OpenWithShell(PathBuf),
            RequestThumbnailLoad(PathBuf, u32),
            RequestFolderScan(PathBuf),
            RequestFolderPreviewLoad(PathBuf),
            RenameWithShell(usize),
        }

        impl ListViewOperations for ListOps<'_> {
            fn navigate_to(&mut self, path: &str) {
                self.actions.push(ListAction::NavigateTo(path.to_string()));
            }

            fn open_with_shell(&mut self, path: &PathBuf) {
                self.actions.push(ListAction::OpenWithShell(path.clone()));
            }

            fn request_thumbnail_load(&mut self, path: PathBuf) {
                // List view always requests small thumbnails (64px)
                self.actions
                    .push(ListAction::RequestThumbnailLoad(path, 64));
            }

            fn request_folder_scan(&mut self, path: PathBuf) {
                self.actions.push(ListAction::RequestFolderScan(path));
            }

            fn request_folder_preview_load(&mut self, path: PathBuf) {
                self.actions
                    .push(ListAction::RequestFolderPreviewLoad(path));
            }

            fn rename_with_shell(&mut self, idx: usize) {
                self.actions.push(ListAction::RenameWithShell(idx));
            }
        }

        let mut ops = ListOps {
            actions: &mut actions,
        };

        let action = list_view::render_list_view(ui, &mut ctx, &mut ops);

        // Update state from context
        self.sort_mode = ctx.sort_mode;
        self.sort_descending = ctx.sort_descending;
        self.renaming_state = ctx.renaming_state;
        self.focus_rename = ctx.focus_rename;

        // Processar ações (bloqueadas durante renomeação)
        let is_renaming = self.renaming_state.is_some();
        match action {
            Some(list_view::ListViewAction::Click(idx)) if !is_renaming => {
                if let Some(item) = self.items.get(idx) {
                    let ctrl = ui.input(|i| i.modifiers.ctrl);
                    let shift = ui.input(|i| i.modifiers.shift);

                    if ctrl {
                        // Ctrl + Click: Toggle item and set focus/anchor
                        if self.multi_selection.contains(&item.path) {
                            self.multi_selection.remove(&item.path);
                        } else {
                            self.multi_selection.insert(item.path.clone());
                        }
                        self.selected_item = Some(idx);
                        self.selection_anchor = Some(idx);
                        self.selected_file = Some(item.clone());
                    } else if shift {
                        // Shift + Click: Range between anchor and click
                        if let Some(anchor) = self.selection_anchor {
                            let (start, end) = if anchor < idx {
                                (anchor, idx)
                            } else {
                                (idx, anchor)
                            };
                            // Add range to selection (Do NOT clear outside selection as requested)
                            for i in start..=end {
                                if let Some(it) = self.items.get(i) {
                                    self.multi_selection.insert(it.path.clone());
                                }
                            }
                            self.selected_item = Some(idx);
                            self.selected_file = Some(item.clone());
                        } else {
                            // Fallback: simple insert
                            self.multi_selection.insert(item.path.clone());
                            self.selected_item = Some(idx);
                            self.selection_anchor = Some(idx);
                            self.selected_file = Some(item.clone());
                        }
                    } else {
                        // Simple Click: Reset selection to target and set focus/anchor
                        self.multi_selection.clear();
                        self.multi_selection.insert(item.path.clone());
                        self.selected_item = Some(idx);
                        self.selection_anchor = Some(idx);
                        self.selected_file = Some(item.clone());
                    }

                    // Common updates
                    let item_path = item.path.clone();
                    let is_dir = item.is_dir;
                    self.update_selected_thumbnail();

                    // Trigger thumbnail load for sidebar preview
                    if !is_dir {
                        if !self.cache_manager.has_thumbnail(&item_path)
                            && !self.cache_manager.is_loading(&item_path)
                        {
                            self.request_thumbnail_load(item_path, 512);
                        }
                    }
                }
            }
            Some(list_view::ListViewAction::DoubleClick(idx)) if !is_renaming => {
                let mut path_to_navigate = None;
                if let Some(item) = self.items.get(idx) {
                    if item.is_dir {
                        if !self.is_recycle_bin_view {
                            path_to_navigate = Some(item.path.clone());
                        }
                    } else {
                        let path = item.path.clone();
                        let extension = path
                            .extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or("")
                            .to_lowercase();
                        if extension == "iso" {
                            self.mount_and_navigate_iso(path);
                        } else {
                            open_with_shell(&path);
                        }
                    }
                }

                if let Some(path) = path_to_navigate {
                    self.navigate_to(&path.to_string_lossy());
                }
            }
            Some(list_view::ListViewAction::SecondaryClick(idx)) if !is_renaming => {
                if let Some(item) = self.items.get(idx) {
                    // Update selection logic for right-click
                    if !self.multi_selection.contains(&item.path) {
                        self.multi_selection.clear();
                        self.multi_selection.insert(item.path.clone());
                        self.selected_item = Some(idx);
                        self.selected_file = Some(item.clone());
                    } else {
                        self.selected_item = Some(idx);
                        self.selected_file = Some(item.clone());
                    }

                    // Coletar todos os paths selecionados
                    let selected_paths: Vec<PathBuf> =
                        self.multi_selection.iter().cloned().collect();
                    self.context_menu.target_paths = selected_paths.clone();

                    // Usar o novo sistema de menu estilizado
                    let pointer_pos = ui.ctx().pointer_latest_pos().unwrap_or(egui::Pos2::ZERO);
                    let right_bound = ui.available_rect_before_wrap().right();

                    // Populate with multiple paths
                    self.populate_context_menu(ui.ctx(), &selected_paths, false, Some(idx));
                    self.context_menu.open(
                        pointer_pos,
                        right_bound,
                        Some(idx),
                        selected_paths,
                        false,
                    );
                }
            }
            Some(list_view::ListViewAction::SortChange(mode)) => {
                // Toggle direction if same mode, otherwise switch mode
                if self.sort_mode == mode {
                    self.sort_descending = !self.sort_descending;
                } else {
                    self.sort_mode = mode;
                    self.sort_descending = false;
                }
                self.sort_items();
                self.save_preferences();
            }
            Some(list_view::ListViewAction::EmptyAreaSecondaryClick) if !is_renaming => {
                let path = PathBuf::from(&self.current_path);
                let pointer_pos = ui.ctx().pointer_latest_pos().unwrap_or(egui::Pos2::ZERO);
                let right_bound = ui.available_rect_before_wrap().right();
                self.populate_context_menu(ui.ctx(), &[path.clone()], true, None);
                self.context_menu
                    .open(pointer_pos, right_bound, None, vec![path], true);
            }
            _ => {}
        }

        // Executar ações coletadas
        for action in actions {
            match action {
                ListAction::NavigateTo(path) => self.navigate_to(&path),
                ListAction::OpenWithShell(path) => open_with_shell(&path),
                ListAction::RequestThumbnailLoad(path, size) => {
                    self.request_thumbnail_load(path, size)
                }
                ListAction::RequestFolderScan(path) => self.request_folder_scan(path),
                ListAction::RequestFolderPreviewLoad(path) => {
                    self.request_folder_preview_load(path)
                }
                ListAction::RenameWithShell(idx) => self.rename_with_shell(idx),
            }
        }

        // Reset scroll trigger after view has consumed it
        self.scroll_to_selected = false;
    }

    // --- GRANDE (GRID VIEW) ---
    pub fn render_grid_view(&mut self, ui: &mut egui::Ui) {
        // Calculate cols for keyboard navigation
        let padding = 8.0;
        let item_w = self.thumbnail_size;
        let item_h = self.thumbnail_size + 20.0;
        let cell_h = item_h + padding;
        let available_w = ui.available_width();
        let cols = ((available_w - padding) / (item_w + padding))
            .floor()
            .max(1.0) as usize;

        // Keyboard navigation (ONLY when not renaming and media is NOT focused)
        if self.renaming_state.is_none() && !self.is_media_keyboard_focused() {
            let current_index = self.items.iter().position(|x| {
                self.selected_file
                    .as_ref()
                    .map_or(false, |f| f.path == x.path)
            });

            let mut pending_delta: i32 = 0;
            let mut page_action: Option<bool> = None;

            if ui.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
                pending_delta += 1;
            }
            if ui.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
                pending_delta -= 1;
            }
            if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
                pending_delta += cols as i32;
            }
            if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
                pending_delta -= cols as i32;
            }
            if ui.input(|i| i.key_pressed(egui::Key::PageDown)) {
                page_action = Some(true);
            }
            if ui.input(|i| i.key_pressed(egui::Key::PageUp)) {
                page_action = Some(false);
            }

            let shift = ui.input(|i| i.modifiers.shift);
            let viewport_h = ui.available_height();
            let visible_rows = (viewport_h / cell_h).floor() as usize;
            let jump = visible_rows * cols;

            let mut new_index = None;
            if let Some(is_down) = page_action {
                if is_down {
                    new_index = Some(
                        current_index
                            .map(|idx| (idx + jump).min(self.items.len().saturating_sub(1)))
                            .unwrap_or(jump),
                    );
                } else {
                    new_index = Some(
                        current_index
                            .map(|idx| idx.saturating_sub(jump))
                            .unwrap_or(0),
                    );
                }
            } else if pending_delta != 0 {
                new_index = Some(
                    current_index
                        .map(|idx| {
                            (idx as i32 + pending_delta)
                                .clamp(0, self.items.len().saturating_sub(1) as i32)
                                as usize
                        })
                        .unwrap_or(0),
                );
            }

            if let Some(idx) = new_index {
                let clamped = idx.min(self.items.len().saturating_sub(1));
                if let Some(item) = self.items.get(clamped) {
                    // Clone path before any mutable borrows
                    let item_path = item.path.clone();

                    // UPDATED: Decoupled Focus (selected_item) from Selection (multi_selection)
                    let old_focus = self.selected_item;
                    self.selected_item = Some(clamped);
                    self.selected_file = Some(item.clone());
                    self.update_selected_thumbnail();
                    self.last_keyboard_nav = Instant::now();

                    if shift {
                        // Shift + Arrow/Page: Range selection
                        if self.selection_anchor.is_none() {
                            self.selection_anchor = old_focus;
                        }
                        if let Some(anchor) = self.selection_anchor {
                            let (start, end) = if anchor < clamped {
                                (anchor, clamped)
                            } else {
                                (clamped, anchor)
                            };
                            // Add range between anchor and focus (NÃO limpar seleção fora do range)
                            for i in start..=end {
                                if let Some(it) = self.items.get(i) {
                                    self.multi_selection.insert(it.path.clone());
                                }
                            }
                        }
                    } else {
                        // Navigation without shift: Single-item selection (clear + add focused item)
                        // This ensures the focused item shows the dark blue selection border
                        self.multi_selection.clear();
                        self.multi_selection.insert(item_path);
                        self.selection_anchor = Some(clamped);
                    }

                    // Trigger scroll normalization in the view
                    self.scroll_to_selected = true;

                    // Request visibility for the new selected index
                    self.scroll_request = crate::app::state::ScrollRequest::EnsureVisible(clamped);

                    ui.ctx().request_repaint();
                }
            }

            // Enter to open (only when not renaming)
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                if let Some(selected) = &self.selected_file.clone() {
                    if selected.is_dir {
                        self.navigate_to(&selected.path.to_string_lossy());
                        return; // Exit early after navigation
                    } else {
                        open_with_shell(&selected.path);
                    }
                }
            }
        }

        // Extrair dados necessários para evitar múltiplos borrows
        let items = self.items.clone(); // Clone para evitar borrow
        let selected_item = self.selected_item;
        let selected_file = self.selected_file.clone();
        let thumbnail_size = self.thumbnail_size;
        let last_grid_cols = self.last_grid_cols;
        let renaming_state = self.renaming_state.clone();
        let focus_rename = self.focus_rename;
        let folder_icon_texture = self.cache_manager.folder_icon_texture.clone();
        let computer_icon = self.cache_manager.computer_icon.clone();

        // Criar contexto com referências mutáveis separadas
        let scroll_to_selected = self.scroll_to_selected;
        let multi_selection = &self.multi_selection;

        // PERFORMANCE: Clear shared buffers before rendering (reuse, don't reallocate)
        self.pending_ops.clear();

        // Check if video is playing in docked mode to reduce disk I/O
        let is_video_playing_docked = self.is_video_playing_docked();

        let mut ctx = GridViewContext {
            items: &items,
            selected_item,
            selected_file: selected_file.as_ref(),
            multi_selection,
            thumbnail_size,
            last_grid_cols,
            renaming_state: renaming_state.clone(),
            focus_rename,
            scroll_to_selected,
            is_computer_view: self.is_computer_view,
            is_recycle_bin_view: self.is_recycle_bin_view,
            texture_cache: &mut self.cache_manager.texture_cache,
            loading_set: &mut self.cache_manager.loading_set,
            loading_icons: &mut self.loading_icons,
            failed_icons: &self.failed_icons,
            scanned_folders: &mut self.scanned_folders,
            folder_icon_texture: folder_icon_texture.as_ref(),
            computer_icon: computer_icon.as_ref(),
            drive_icon_cache: &mut self.cache_manager.drive_icon_cache,
            item_icon_loader: &mut self.item_icon_loader,
            folder_preview_cache: &mut self.cache_manager.folder_preview_cache,
            folder_preview_loading: &mut self.cache_manager.folder_preview_loading,
            pending_ops: &mut self.pending_ops,
            failed_thumbnails: &self.cache_manager.failed_thumbnails,
            scroll_offset_y: self.scroll_offset_y,
            mut_scroll_offset_y: &mut self.scroll_offset_y,
            last_input: self.last_input,
            last_scroll_time: &mut self.last_scroll_time,
            last_scroll_offset: &mut self.last_scroll_offset,
            pending_upload_set: &mut self.cache_manager.pending_upload_set,
            is_video_playing_docked,
        };

        // Usar uma abordagem diferente: coletar ações em vetores
        let mut actions = Vec::new();

        struct GridOps<'a> {
            actions: &'a mut Vec<GridAction>,
        }

        enum GridAction {
            NavigateTo(String),
            OpenWithShell(PathBuf),
            RequestThumbnailLoad(PathBuf, u32),
            RequestFolderScan(PathBuf),
            RequestFolderPreviewLoad(PathBuf),
            RequestThumbnailPrefetch(PathBuf, u32),
            RequestIconLoad(PathBuf),
            RenameWithShell(usize),
        }

        impl GridViewOperations for GridOps<'_> {
            fn navigate_to(&mut self, path: &str) {
                self.actions.push(GridAction::NavigateTo(path.to_string()));
            }

            fn open_with_shell(&mut self, path: &PathBuf) {
                self.actions.push(GridAction::OpenWithShell(path.clone()));
            }

            fn request_thumbnail_load(&mut self, path: PathBuf, size: u32) {
                self.actions
                    .push(GridAction::RequestThumbnailLoad(path, size));
            }

            fn request_folder_scan(&mut self, path: PathBuf) {
                self.actions.push(GridAction::RequestFolderScan(path));
            }
            fn request_folder_preview_load(&mut self, path: PathBuf) {
                self.actions
                    .push(GridAction::RequestFolderPreviewLoad(path));
            }

            fn request_thumbnail_prefetch(&mut self, path: PathBuf, size: u32) {
                self.actions
                    .push(GridAction::RequestThumbnailPrefetch(path, size));
            }

            fn request_icon_load(&mut self, path: PathBuf) {
                self.actions.push(GridAction::RequestIconLoad(path));
            }

            fn rename_with_shell(&mut self, idx: usize) {
                self.actions.push(GridAction::RenameWithShell(idx));
            }
        }

        let mut ops = GridOps {
            actions: &mut actions,
        };

        let action = grid_view::render_grid_view(ui, &mut ctx, &mut ops);

        // Update state from context
        self.last_grid_cols = ctx.last_grid_cols;
        self.renaming_state = ctx.renaming_state;
        self.focus_rename = ctx.focus_rename;

        // Processar ações (bloqueadas durante renomeação, exceto clique no próprio item)
        let is_renaming = self.renaming_state.is_some();
        match action {
            Some(grid_view::GridViewAction::Click(idx)) if !is_renaming => {
                if let Some(item) = self.items.get(idx) {
                    let ctrl = ui.input(|i| i.modifiers.ctrl);
                    let shift = ui.input(|i| i.modifiers.shift);

                    if ctrl {
                        // Ctrl + Click: Toggle and set focus/anchor
                        if self.multi_selection.contains(&item.path) {
                            self.multi_selection.remove(&item.path);
                        } else {
                            self.multi_selection.insert(item.path.clone());
                        }
                        self.selected_item = Some(idx);
                        self.selection_anchor = Some(idx);
                        self.selected_file = Some(item.clone());
                    } else if shift {
                        // Shift + Click: Range between anchor and click
                        if let Some(anchor) = self.selection_anchor {
                            let (start, end) = if anchor < idx {
                                (anchor, idx)
                            } else {
                                (idx, anchor)
                            };
                            for i in start..=end {
                                if let Some(it) = self.items.get(i) {
                                    self.multi_selection.insert(it.path.clone());
                                }
                            }
                            self.selected_item = Some(idx);
                            self.selected_file = Some(item.clone());
                        } else {
                            // Fallback
                            self.multi_selection.insert(item.path.clone());
                            self.selected_item = Some(idx);
                            self.selection_anchor = Some(idx);
                            self.selected_file = Some(item.clone());
                        }
                    } else {
                        // Simple Click: Reset selection and set focus/anchor
                        self.multi_selection.clear();
                        self.multi_selection.insert(item.path.clone());
                        self.selected_item = Some(idx);
                        self.selection_anchor = Some(idx);
                        self.selected_file = Some(item.clone());
                    }

                    self.update_selected_thumbnail();
                }
            }
            Some(grid_view::GridViewAction::DoubleClick(idx)) if !is_renaming => {
                let mut path_to_navigate = None;
                if let Some(item) = self.items.get(idx) {
                    if item.is_dir {
                        if !self.is_recycle_bin_view {
                            path_to_navigate = Some(item.path.clone());
                        }
                    } else {
                        let path = item.path.clone();
                        let extension = path
                            .extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or("")
                            .to_lowercase();
                        if extension == "iso" {
                            self.mount_and_navigate_iso(path);
                        } else {
                            open_with_shell(&path);
                        }
                    }
                }

                if let Some(path) = path_to_navigate {
                    self.navigate_to(&path.to_string_lossy());
                }
            }
            Some(grid_view::GridViewAction::SecondaryClick(idx)) if !is_renaming => {
                if let Some(item) = self.items.get(idx) {
                    // Update selection logic for right-click
                    if !self.multi_selection.contains(&item.path) {
                        self.multi_selection.clear();
                        self.multi_selection.insert(item.path.clone());
                        self.selected_item = Some(idx);
                        self.selected_file = Some(item.clone());
                    } else {
                        self.selected_item = Some(idx);
                        self.selected_file = Some(item.clone());
                    }

                    // Coletar todos os paths selecionados
                    let selected_paths: Vec<PathBuf> =
                        self.multi_selection.iter().cloned().collect();
                    self.context_menu.target_paths = selected_paths.clone();

                    // Usar o novo sistema de menu estilizado
                    let pointer_pos = ui.ctx().pointer_latest_pos().unwrap_or(egui::Pos2::ZERO);
                    let right_bound = ui.available_rect_before_wrap().right();

                    self.populate_context_menu(ui.ctx(), &selected_paths, false, Some(idx));
                    self.context_menu.open(
                        pointer_pos,
                        right_bound,
                        Some(idx),
                        selected_paths,
                        false,
                    );
                }
            }
            Some(grid_view::GridViewAction::EmptyAreaSecondaryClick) if !is_renaming => {
                let path = PathBuf::from(&self.current_path);
                let pointer_pos = ui.ctx().pointer_latest_pos().unwrap_or(egui::Pos2::ZERO);
                let right_bound = ui.available_rect_before_wrap().right();
                self.populate_context_menu(ui.ctx(), &[path.clone()], true, None);
                self.context_menu
                    .open(pointer_pos, right_bound, None, vec![path], true);
            }
            _ => {}
        }

        // Executar ações coletadas
        for action in actions {
            match action {
                GridAction::NavigateTo(path) => self.navigate_to(&path),
                GridAction::OpenWithShell(path) => open_with_shell(&path),
                GridAction::RequestThumbnailLoad(path, size) => {
                    self.request_thumbnail_load(path, size)
                }
                GridAction::RequestFolderScan(path) => self.request_folder_scan(path),
                GridAction::RequestFolderPreviewLoad(path) => {
                    self.request_folder_preview_load(path)
                }
                GridAction::RequestThumbnailPrefetch(path, size) => {
                    self.request_thumbnail_prefetch(path, size)
                }
                GridAction::RequestIconLoad(path) => self.request_icon_load(path),
                GridAction::RenameWithShell(idx) => self.rename_with_shell(idx),
            }
        }

        // Reset scroll trigger after view has consumed it
        self.scroll_to_selected = false;
    }

    pub fn render_item_slot(&mut self, ui: &mut egui::Ui, idx: usize) {
        if idx >= self.items.len() {
            return;
        }

        // Clone Arc to avoid borrowing self.items, allowing us to borrow the item
        // without a deep clone while still mutating self later
        let items_arc = self.items.clone();
        let item = &items_arc[idx];
        let is_renaming = self
            .renaming_state
            .as_ref()
            .map_or(false, |(i, _)| *i == idx);

        // Para evitar conflitos de borrow, coletamos as operações pendentes
        // e executamos depois de renderizar
        let mut pending_thumbnail_loads: Vec<(std::path::PathBuf, u32)> = Vec::new();
        let mut pending_folder_scans: Vec<std::path::PathBuf> = Vec::new();
        let mut pending_folder_preview_loads: Vec<std::path::PathBuf> = Vec::new();
        let mut pending_icon_loads: Vec<std::path::PathBuf> = Vec::new();
        let mut pending_rename: Option<usize> = None;

        // Texto de renomeação precisa ser tratado separadamente
        let mut renaming_text_clone = if is_renaming {
            self.renaming_state.as_ref().map(|(_, s)| s.clone())
        } else {
            None
        };

        // Create context with mutable reference to the clone
        {
            let renaming_text = renaming_text_clone.as_mut();

            let mut ctx = ItemSlotContext {
                item,
                idx,
                thumbnail_size: self.thumbnail_size,
                is_renaming,
                renaming_text,
                focus_rename: self.focus_rename,
                is_recycle_bin_view: self.is_recycle_bin_view,
                texture_cache: &mut self.cache_manager.texture_cache,
                icon_loader: &mut self.item_icon_loader,
                scanned_folders: &mut self.scanned_folders,
                loading_set: &mut self.cache_manager.loading_set,
                loading_icons: &mut self.loading_icons,
                failed_icons: &self.failed_icons,
                folder_preview_cache: &mut self.cache_manager.folder_preview_cache,
                folder_preview_loading: &mut self.cache_manager.folder_preview_loading,
                failed_thumbnails: &self.cache_manager.failed_thumbnails,
                pending_upload_set: &mut self.cache_manager.pending_upload_set,
                is_dense_mode: false,
            };

            // Create simple ops struct that collects operations
            struct SimpleOps<'a> {
                thumbnail_loads: &'a mut Vec<(std::path::PathBuf, u32)>,
                folder_scans: &'a mut Vec<std::path::PathBuf>,
                folder_preview_loads: &'a mut Vec<std::path::PathBuf>,
                icon_loads: &'a mut Vec<std::path::PathBuf>,
                pending_rename: &'a mut Option<usize>,
            }

            impl<'a> crate::ui::components::item_slot::ItemSlotOperations for SimpleOps<'a> {
                fn request_thumbnail_load(&mut self, path: std::path::PathBuf, size: u32) {
                    self.thumbnail_loads.push((path, size));
                }

                fn request_folder_scan(&mut self, path: std::path::PathBuf) {
                    self.folder_scans.push(path);
                }

                fn request_folder_preview_load(&mut self, path: std::path::PathBuf) {
                    self.folder_preview_loads.push(path);
                }

                fn request_icon_load(&mut self, path: std::path::PathBuf) {
                    self.icon_loads.push(path);
                }

                fn rename_item(&mut self, idx: usize) {
                    *self.pending_rename = Some(idx);
                }
            }

            let mut ops = SimpleOps {
                thumbnail_loads: &mut pending_thumbnail_loads,
                folder_scans: &mut pending_folder_scans,
                folder_preview_loads: &mut pending_folder_preview_loads,
                icon_loads: &mut pending_icon_loads,
                pending_rename: &mut pending_rename,
            };

            render_item_slot(ui, &mut ctx, &mut ops);
        }

        // Apply changes after render
        if let Some(new_text) = renaming_text_clone {
            if is_renaming {
                if let Some((_, ref mut text)) = self.renaming_state {
                    *text = new_text;
                }
            }
        }

        // Execute pending operations
        for (path, size) in pending_thumbnail_loads {
            self.request_thumbnail_load(path, size);
        }

        for path in pending_folder_scans {
            self.request_folder_scan(path);
        }

        for path in pending_folder_preview_loads {
            self.request_folder_preview_load(path);
        }

        for path in pending_icon_loads {
            self.request_icon_load(path);
        }

        if let Some(rename_idx) = pending_rename {
            self.rename_with_shell(rename_idx);
        }

        // Reset focus flag after first use
        if self.focus_rename {
            self.focus_rename = false;
        }
    }
}
