//! UI Rendering functions: list_view, grid_view, item_slot
//!
//! This module contains the main logic for rendering the file lists in different modes.
//! It bridges the App state with the UI components.

use eframe::egui;
use std::time::{Duration, Instant};
use std::path::{Path, PathBuf};

use crate::app::state::ImageViewerApp;
use crate::ui::views::{list_view, ListViewContext, ListViewOperations};
use crate::ui::views::{grid_view, GridViewContext, GridViewOperations};
use crate::ui::components::item_slot::{render_item_slot, ItemSlotContext};

// Helper function equivalent to open_with_shell from ops
fn open_with_shell(path: &Path) {
     let _ = crate::application::file_operations::open_with_shell(path, None);
}

impl ImageViewerApp {
    // --- DETALHES (LIST VIEW) ---
    pub fn render_list_view(&mut self, ui: &mut egui::Ui) {
        // Keyboard navigation for list view (ONLY when not renaming)
        // Throttle: 50ms between navigations to prevent scroll desync when holding keys
        if self.renaming_state.is_none()
            && self.last_keyboard_nav.elapsed() >= Duration::from_millis(50)
        {
            let current_index = self.items.iter().position(|x| {
                self.selected_file
                    .as_ref()
                    .map_or(false, |f| f.path == x.path)
            });

            let mut new_index = None;
            if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
                new_index = current_index.map(|idx| idx + 1).or(Some(0));
            } else if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
                new_index = current_index.map(|idx| idx.saturating_sub(1));
            }

            if let Some(idx) = new_index {
                let clamped = idx.min(self.items.len().saturating_sub(1));
                if let Some(item) = self.items.get(clamped) {
                    let item_path = item.path.clone();
                    let is_dir = item.is_dir;

                    self.selected_file = Some(item.clone());
                    self.selected_item = Some(clamped);
                    self.update_selected_thumbnail();
                    self.scroll_to_selected = true; // Trigger scroll to selected item
                    self.last_keyboard_nav = Instant::now(); // Reset throttle timer

                    // Trigger thumbnail load for sidebar preview
                    if !is_dir {
                        if !self.cache_manager.has_thumbnail(&item_path)
                            && !self.cache_manager.is_loading(&item_path)
                        {
                            self.request_thumbnail_load(item_path);
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
        let mut ctx = ListViewContext {
            items: &items,
            selected_item,
            selected_file: selected_file.as_ref(),
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
            scanned_folders: &mut self.scanned_folders,
            folder_icon_texture: folder_icon_texture.as_ref(),
            computer_icon: computer_icon.as_ref(),
            drive_icon_cache: &mut self.cache_manager.drive_icon_cache,
            item_icon_loader: &mut self.item_icon_loader,
            deletion_date_cache: Some(&mut self.deletion_date_cache),
        };

        // Usar uma abordagem diferente: coletar ações em vetores
        let mut actions = Vec::new();

        struct ListOps<'a> {
            actions: &'a mut Vec<ListAction>,
        }

        enum ListAction {
            NavigateTo(String),
            OpenWithShell(PathBuf),
            RequestThumbnailLoad(PathBuf),
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
                self.actions.push(ListAction::RequestThumbnailLoad(path));
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
        self.scroll_to_selected = false; // Reset after scrolling

        // Processar ações (bloqueadas durante renomeação)
        let is_renaming = self.renaming_state.is_some();
        match action {
            Some(list_view::ListViewAction::Click(idx)) if !is_renaming => {
                self.selected_item = Some(idx);
                if let Some(item) = self.items.get(idx) {
                    let item_path = item.path.clone();
                    let is_dir = item.is_dir;

                    self.selected_file = Some(item.clone());
                    self.update_selected_thumbnail();

                    // Trigger thumbnail load for sidebar preview
                    if !is_dir {
                        if !self.cache_manager.has_thumbnail(&item_path)
                            && !self.cache_manager.is_loading(&item_path)
                        {
                            self.request_thumbnail_load(item_path);
                        }
                    }
                }
            }
            Some(list_view::ListViewAction::DoubleClick(idx)) if !is_renaming => {
                let path_to_navigate = self.items.get(idx).map(|item| {
                    if item.is_dir {
                        if self.is_recycle_bin_view {
                            return None;
                        }
                        Some(item.path.clone())
                    } else {
                        open_with_shell(&item.path);
                        None
                    }
                });

                if let Some(Some(path)) = path_to_navigate {
                    self.navigate_to(&path.to_string_lossy());
                }
            }
            Some(list_view::ListViewAction::SecondaryClick(idx)) if !is_renaming => {
                // Step 1: Update selection immediately (this will cause a repaint)
                self.selected_item = Some(idx);
                if let Some(item) = self.items.get(idx) {
                    let item_path = item.path.clone();
                    self.selected_file = Some(item.clone());
                    self.context_menu.target_path = Some(item_path.clone());

                    // Usar o novo sistema de menu estilizado
                    let pointer_pos = ui.ctx().pointer_latest_pos().unwrap_or(egui::Pos2::ZERO);
                    self.populate_context_menu(ui.ctx(), &item_path, false, Some(idx));
                    self.context_menu
                        .open(pointer_pos, Some(idx), Some(item_path), false);
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
            _ => {}
        }

        // Executar ações coletadas
        for action in actions {
            match action {
                ListAction::NavigateTo(path) => self.navigate_to(&path),
                ListAction::OpenWithShell(path) => open_with_shell(&path),
                ListAction::RequestThumbnailLoad(path) => self.request_thumbnail_load(path),
                ListAction::RequestFolderScan(path) => self.request_folder_scan(path),
                ListAction::RequestFolderPreviewLoad(path) => {
                    self.request_folder_preview_load(path)
                }
                ListAction::RenameWithShell(idx) => self.rename_with_shell(idx),
            }
        }
    }

    // --- GRANDE (GRID VIEW) ---
    pub fn render_grid_view(&mut self, ui: &mut egui::Ui) {
        // Calculate cols for keyboard navigation
        let padding = 8.0;
        let item_w = self.thumbnail_size;
        let available_w = ui.available_width();
        let cols = ((available_w - padding) / (item_w + padding))
            .floor()
            .max(1.0) as usize;

        // Keyboard navigation (ONLY when not renaming)
        // Throttle: 50ms between navigations to prevent scroll desync when holding keys
        if self.renaming_state.is_none()
            && self.last_keyboard_nav.elapsed() >= Duration::from_millis(50)
        {
            let current_index = self.items.iter().position(|x| {
                self.selected_file
                    .as_ref()
                    .map_or(false, |f| f.path == x.path)
            });

            let mut new_index = None;
            if ui.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
                new_index = current_index.map(|idx| idx + 1).or(Some(0));
            } else if ui.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
                new_index = current_index.map(|idx| idx.saturating_sub(1));
            } else if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
                new_index = current_index.map(|idx| idx + cols).or(Some(0));
            } else if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
                new_index = current_index.map(|idx| idx.saturating_sub(cols));
            }

            if let Some(idx) = new_index {
                let clamped = idx.min(self.items.len().saturating_sub(1));
                if let Some(item) = self.items.get(clamped) {
                    self.selected_file = Some(item.clone());
                    self.selected_item = Some(clamped);
                    self.update_selected_thumbnail();
                    self.scroll_to_selected = true; // Trigger scroll to selected item
                    self.last_keyboard_nav = Instant::now(); // Reset throttle timer
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
        let mut ctx = GridViewContext {
            items: &items,
            selected_item,
            selected_file: selected_file.as_ref(),
            thumbnail_size,
            last_grid_cols,
            renaming_state: renaming_state.clone(),
            focus_rename,
            scroll_to_selected,
            is_computer_view: self.is_computer_view,
            is_recycle_bin_view: self.is_recycle_bin_view,
            texture_cache: &mut self.cache_manager.texture_cache,
            loading_set: &mut self.cache_manager.loading_set,
            scanned_folders: &mut self.scanned_folders,
            folder_icon_texture: folder_icon_texture.as_ref(),
            computer_icon: computer_icon.as_ref(),
            drive_icon_cache: &mut self.cache_manager.drive_icon_cache,
            item_icon_loader: &mut self.item_icon_loader,
            folder_preview_cache: &mut self.cache_manager.folder_preview_cache,
            folder_preview_loading: &mut self.cache_manager.folder_preview_loading,
        };

        // Usar uma abordagem diferente: coletar ações em vetores
        let mut actions = Vec::new();

        struct GridOps<'a> {
            actions: &'a mut Vec<GridAction>,
        }

        enum GridAction {
            NavigateTo(String),
            OpenWithShell(PathBuf),
            RequestThumbnailLoad(PathBuf),
            RequestFolderScan(PathBuf),
            RequestFolderPreviewLoad(PathBuf),
            RenameWithShell(usize),
        }

        impl GridViewOperations for GridOps<'_> {
            fn navigate_to(&mut self, path: &str) {
                self.actions.push(GridAction::NavigateTo(path.to_string()));
            }

            fn open_with_shell(&mut self, path: &PathBuf) {
                self.actions.push(GridAction::OpenWithShell(path.clone()));
            }

            fn request_thumbnail_load(&mut self, path: PathBuf) {
                self.actions.push(GridAction::RequestThumbnailLoad(path));
            }

            fn request_folder_scan(&mut self, path: PathBuf) {
                self.actions.push(GridAction::RequestFolderScan(path));
            }
            fn request_folder_preview_load(&mut self, path: PathBuf) {
                self.actions
                    .push(GridAction::RequestFolderPreviewLoad(path));
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
        self.scroll_to_selected = false; // Reset after scrolling

        // Processar ações (bloqueadas durante renomeação, exceto clique no próprio item)
        let is_renaming = self.renaming_state.is_some();
        match action {
            Some(grid_view::GridViewAction::Click(idx)) if !is_renaming => {
                self.selected_item = Some(idx);
                if let Some(item) = self.items.get(idx) {
                    self.selected_file = Some(item.clone());
                    self.update_selected_thumbnail();
                }
            }
            Some(grid_view::GridViewAction::DoubleClick(idx)) if !is_renaming => {
                let path_to_navigate = self.items.get(idx).map(|item| {
                    if item.is_dir {
                        if self.is_recycle_bin_view {
                            return None;
                        }
                        Some(item.path.clone())
                    } else {
                        open_with_shell(&item.path);
                        None
                    }
                });

                if let Some(Some(path)) = path_to_navigate {
                    self.navigate_to(&path.to_string_lossy());
                }
            }
            Some(grid_view::GridViewAction::SecondaryClick(idx)) if !is_renaming => {
                // Step 1: Update selection immediately (this will cause a repaint)
                self.selected_item = Some(idx);
                if let Some(item) = self.items.get(idx) {
                    let item_path = item.path.clone();
                    self.selected_file = Some(item.clone());
                    self.context_menu.target_path = Some(item_path.clone());

                    // Usar o novo sistema de menu estilizado
                    let pointer_pos = ui.ctx().pointer_latest_pos().unwrap_or(egui::Pos2::ZERO);
                    self.populate_context_menu(ui.ctx(), &item_path, false, Some(idx));
                    self.context_menu
                        .open(pointer_pos, Some(idx), Some(item_path), false);
                }
            }
            _ => {}
        }

        // Executar ações coletadas
        for action in actions {
            match action {
                GridAction::NavigateTo(path) => self.navigate_to(&path),
                GridAction::OpenWithShell(path) => open_with_shell(&path),
                GridAction::RequestThumbnailLoad(path) => self.request_thumbnail_load(path),
                GridAction::RequestFolderScan(path) => self.request_folder_scan(path),
                GridAction::RequestFolderPreviewLoad(path) => {
                    self.request_folder_preview_load(path)
                }
                GridAction::RenameWithShell(idx) => self.rename_with_shell(idx),
            }
        }
    }

    pub fn render_item_slot(&mut self, ui: &mut egui::Ui, idx: usize) {
        if idx >= self.items.len() {
            return;
        }

        // Clone item data to avoid borrowing self.items during the render
        let item = self.items[idx].clone();
        let is_renaming = self
            .renaming_state
            .as_ref()
            .map_or(false, |(i, _)| *i == idx);

        // Para evitar conflitos de borrow, coletamos as operações pendentes
        // e executamos depois de renderizar
        let mut pending_thumbnail_loads: Vec<std::path::PathBuf> = Vec::new();
        let mut pending_folder_scans: Vec<std::path::PathBuf> = Vec::new();
        let mut pending_folder_preview_loads: Vec<std::path::PathBuf> = Vec::new();
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
                item: &item,
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
                folder_preview_cache: &mut self.cache_manager.folder_preview_cache,
                folder_preview_loading: &mut self.cache_manager.folder_preview_loading,
            };

            // Create simple ops struct that collects operations
            struct SimpleOps<'a> {
                thumbnail_loads: &'a mut Vec<std::path::PathBuf>,
                folder_scans: &'a mut Vec<std::path::PathBuf>,
                folder_preview_loads: &'a mut Vec<std::path::PathBuf>,
                pending_rename: &'a mut Option<usize>,
            }

            impl<'a> crate::ui::components::item_slot::ItemSlotOperations for SimpleOps<'a> {
                fn request_thumbnail_load(&mut self, path: std::path::PathBuf) {
                    self.thumbnail_loads.push(path);
                }

                fn request_folder_scan(&mut self, path: std::path::PathBuf) {
                    self.folder_scans.push(path);
                }

                fn request_folder_preview_load(&mut self, path: std::path::PathBuf) {
                    self.folder_preview_loads.push(path);
                }

                fn rename_item(&mut self, idx: usize) {
                    *self.pending_rename = Some(idx);
                }
            }

            let mut ops = SimpleOps {
                thumbnail_loads: &mut pending_thumbnail_loads,
                folder_scans: &mut pending_folder_scans,
                folder_preview_loads: &mut pending_folder_preview_loads,
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
        for path in pending_thumbnail_loads {
            ImageViewerApp::request_thumbnail_load(&*self, path);
        }

        for path in pending_folder_scans {
            ImageViewerApp::request_folder_scan(&*self, path);
        }

        for path in pending_folder_preview_loads {
            // Need to implement this in self or import it
            self.request_folder_preview_load(path);
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
