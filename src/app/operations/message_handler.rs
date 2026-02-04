//! Async message processing from workers
//!
//! This module processes incoming messages from various background workers
//! (filesystem events, thumbnails, folder sizes, etc.) and updates the UI state.

use crate::app::state::{ImageViewerApp, ItemsRebuildResult};
use crate::application::sorting;
use crate::ui::theme;
use eframe::egui;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

impl ImageViewerApp {
    pub fn process_incoming_messages(&mut self, ctx: &egui::Context) {
        // 1. CHECK DE REFRESH MANUAL (F5)
        if ctx.input(|i| i.key_pressed(egui::Key::F5)) {
            self.trigger_manual_refresh();
        }

        let mut saw_device_event = false;
        while self.device_event_receiver.try_recv().is_ok() {
            saw_device_event = true;
        }

        if saw_device_event {
            // Drive was inserted/removed: clear all drive icon caches so icons are re-extracted
            self.item_icon_loader.clear_drive_icons();

            let old_disks = self.disks.clone();
            if self.reload_drive_list() {
                self.last_drive_refresh = Instant::now();

                // AUTO-FOCUS PARA ISO RECÉM-MONTADA
                if let Some(_iso_path) = self.pending_iso_mount.take() {
                    let mut target_drive = None;
                    // Encontra qual drive é novo
                    for (new_path, _label) in &self.disks {
                        if !old_disks.iter().any(|(old_path, _)| old_path == new_path) {
                            // VERIFICAÇÃO: O drive realmente está pronto/acessível?
                            if std::path::Path::new(new_path).exists() {
                                target_drive = Some(new_path.clone());
                                break;
                            }
                        }
                    }

                    if let Some(drive) = target_drive {
                        // Navega para ele!
                        self.navigate_to(&drive);
                    } else {
                        // Se não encontrou drive válido, devolve para o estado pendente
                        // para tentar no próximo evento (pode ser que o Windows mande vários)
                        self.pending_iso_mount = Some(_iso_path);
                    }
                }

                if self.is_computer_view {
                    self.setup_computer_view();
                }
                // Force immediate repaint without waiting for input events
                ctx.request_repaint_after(std::time::Duration::from_millis(0));
            }
        }

        // Apply async rebuild results (filter/sort) from background thread
        // BLOCKING: Process all available results in batch
        loop {
            match self.items_rebuild_receiver.try_recv() {
                Ok(result) => {
                    if result.generation != self.generation {
                        continue;
                    }
                    if result.request_id != self.items_rebuild_request_id {
                        continue;
                    }
                    self.items = Arc::new(result.items);
                    self.total_items = result.total_items;

                    // After rebuild: if a pending selection was requested (e.g., after rename),
                    // find the item and select + scroll to it.
                    if let Some(target_path) = self.pending_select_path.take() {
                        if let Some(idx) = self.items.iter().position(|i| i.path == target_path) {
                            self.selected_item = Some(idx);
                            self.selected_file = Some(self.items[idx].clone());
                            self.scroll_to_selected = true;
                        }
                    }

                    ctx.request_repaint();
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break, // No more messages
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            }
        }

        fn normalize_for_match(p: &Path) -> String {
            let s = p.to_string_lossy().to_string().to_lowercase();
            if let Some(stripped) = s.strip_prefix(r"\\?\") {
                stripped.to_string()
            } else {
                s
            }
        }

        // BLOCKING: Process all available file operation results in batch
        loop {
            match self.file_op_res_receiver.try_recv() {
                Ok(res) => {
                    use crate::workers::file_operation_worker::FileOperationResult;
                    match res {
                FileOperationResult::RenameCompleted {
                    path,
                    new_name,
                    parent_folder,
                } => {
                    let current_str = normalize_for_match(Path::new(&self.current_path));
                    let parent_str = normalize_for_match(parent_folder.as_path());
                    // PERFORMANCE: Cache path normalization to avoid redundant allocations
                    let path_str = normalize_for_match(&path);
                    self.directory_cache.invalidate(&parent_folder);
                    if let Some(di) = &self.directory_index {
                        let _ = di.invalidate(&parent_folder);
                    }

                    // FIX: If the renamed item is currently selected, update the selection state
                    // This prevents stale data in the Details Panel even before reload completes.
                    // Note: load_folder() does NOT clear selected_file, so this persists correctly.
                    if let Some(selected) = &mut self.selected_file {
                        if normalize_for_match(&selected.path) == path_str {
                            let new_path = parent_folder.join(&new_name);
                            selected.path = new_path;
                            selected.name = new_name.clone();
                        }
                    }

                    // FIX: Stop media player if the renamed file is currently playing.
                    // The player holds the OLD path, so the preview panel would show a
                    // broken state (thumbnail over playing video, no controls).
                    if let Some(crate::ui::components::media_preview::MediaPreview::Video(ref mut player)) = self.media_preview {
                        if normalize_for_match(&player.path) == path_str {
                            player.pause();
                            self.media_preview = None;
                            self.media_preview_owner_tab_id = None;
                        }
                    }

                    for tab in self.tab_manager.tabs.iter_mut() {
                        let tab_path = normalize_for_match(Path::new(&tab.path));
                        if tab_path == parent_str {
                            tab.items = std::sync::Arc::new(Vec::new());
                            tab.all_items.clear();
                        }
                    }
                    if parent_str == current_str {
                        // After reload + re-sort, select and scroll to the renamed item
                        let new_path = parent_folder.join(&new_name);
                        self.pending_select_path = Some(new_path);
                        self.loaded_path.clear();
                        self.load_folder(false);
                    }
                }
                FileOperationResult::RecycleBinChanged => {
                    if self.is_recycle_bin_view {
                        eprintln!("[RECYCLE] Operation finished, refreshing view.");
                        self.setup_recycle_bin_view();
                        // CRITICAL: Sync back to tab so tab_manager knows we are still in Lixeira
                        self.sync_to_tab();
                    }
                }
                FileOperationResult::RestoreCompleted { parent_folders } => {
                    let current_str = normalize_for_match(Path::new(&self.current_path));
                    let mut should_reload_current = false;

                    for parent in parent_folders {
                        self.directory_cache.invalidate(&parent);
                        if let Some(di) = &self.directory_index {
                            let _ = di.invalidate(&parent);
                        }

                        let parent_str = normalize_for_match(parent.as_path());
                        if parent_str == current_str {
                            should_reload_current = true;
                        }

                        for tab in self.tab_manager.tabs.iter_mut() {
                            let tab_path = normalize_for_match(Path::new(&tab.path));
                            if tab_path == parent_str {
                                tab.items = std::sync::Arc::new(Vec::new());
                                tab.all_items.clear();
                            }
                        }
                    }

                    if should_reload_current {
                        self.loaded_path.clear();
                        self.load_folder(false);
                    }
                }
                FileOperationResult::DeleteCompleted { parent_folders } => {
                    let current_str = normalize_for_match(Path::new(&self.current_path));
                    let mut should_reload_current = false;
                    for parent in parent_folders {
                        self.directory_cache.invalidate(&parent);
                        if let Some(di) = &self.directory_index {
                            let _ = di.invalidate(&parent);
                        }
                        let parent_str = normalize_for_match(parent.as_path());
                        if parent_str == current_str {
                            should_reload_current = true;
                        }
                        for tab in self.tab_manager.tabs.iter_mut() {
                            let tab_path = normalize_for_match(Path::new(&tab.path));
                            if tab_path == parent_str {
                                tab.items = std::sync::Arc::new(Vec::new());
                                tab.all_items.clear();
                            }
                        }
                    }
                    if should_reload_current {
                        self.loaded_path.clear();
                        self.load_folder(false);
                    }
                }
                FileOperationResult::CopyCompleted { dest_folder } => {
                    let dest_str = normalize_for_match(dest_folder.as_path());
                    let current_str = normalize_for_match(Path::new(&self.current_path));

                    self.directory_cache.invalidate(&dest_folder);
                    if let Some(di) = &self.directory_index {
                        let _ = di.invalidate(&dest_folder);
                    }
                    for tab in self.tab_manager.tabs.iter_mut() {
                        let tab_path = normalize_for_match(Path::new(&tab.path));
                        if tab_path == dest_str {
                            tab.items = std::sync::Arc::new(Vec::new());
                            tab.all_items.clear();
                        }
                    }

                    // Clear thumbnail failure caches so files that failed extraction
                    // during copy (locked by Windows) get retried now that copy is done
                    self.cache_manager.clear_failed();
                    crate::workers::thumbnail::clear_all_failures();

                    if dest_str == current_str {
                        eprintln!(
                            "[COPY] Dest folder matches current view, reloading: {}",
                            self.current_path
                        );
                        self.loaded_path.clear();
                        self.load_folder(false);
                    }
                }
                FileOperationResult::MoveCompleted {
                    source_folder,
                    dest_folder,
                } => {
                    let source_str = normalize_for_match(source_folder.as_path());
                    let dest_str = normalize_for_match(dest_folder.as_path());
                    let current_str = normalize_for_match(Path::new(&self.current_path));

                    // 1. Source Logic (Item Removed)
                    self.directory_cache.invalidate(&source_folder);
                    if let Some(di) = &self.directory_index {
                        let _ = di.invalidate(&source_folder);
                    }
                    self.directory_cache.invalidate(&dest_folder);
                    if let Some(di) = &self.directory_index {
                        let _ = di.invalidate(&dest_folder);
                    }

                    // Clear thumbnail failure caches for retry after move completes
                    self.cache_manager.clear_failed();
                    crate::workers::thumbnail::clear_all_failures();

                    if current_str == source_str {
                        eprintln!(
                            "[MOVE] Source folder matches current view, reloading: {}",
                            self.current_path
                        );
                        self.loaded_path.clear();
                        self.load_folder(false);
                    }

                    // Also update cached items in other tabs pointing to this folder
                    for tab in self.tab_manager.tabs.iter_mut() {
                        let tab_path = normalize_for_match(Path::new(&tab.path));
                        if tab_path == source_str || tab_path == dest_str {
                            tab.items = std::sync::Arc::new(Vec::new());
                            tab.all_items.clear();
                        }
                    }

                    // 2. Destination Logic (Item Added)
                    if current_str == dest_str {
                        eprintln!(
                            "[MOVE] Dest folder matches current view, reloading: {}",
                            self.current_path
                        );
                        self.loaded_path.clear();
                        self.load_folder(false);
                    }
                }
                FileOperationResult::MoveBatchCompleted {
                    source_folders,
                    dest_folder,
                } => {
                    let dest_str = normalize_for_match(dest_folder.as_path());
                    let current_str = normalize_for_match(Path::new(&self.current_path));

                    // Clear thumbnail failure caches for retry after move completes
                    self.cache_manager.clear_failed();
                    crate::workers::thumbnail::clear_all_failures();

                    // Invalidate all source folders and destination
                    for source_folder in &source_folders {
                        self.directory_cache.invalidate(source_folder);
                        if let Some(di) = &self.directory_index {
                            let _ = di.invalidate(source_folder);
                        }
                    }
                    self.directory_cache.invalidate(&dest_folder);
                    if let Some(di) = &self.directory_index {
                        let _ = di.invalidate(&dest_folder);
                    }

                    // Check if current view matches any source folder
                    let mut should_reload = false;
                    for source_folder in &source_folders {
                        let source_str = normalize_for_match(source_folder.as_path());
                        if current_str == source_str {
                            should_reload = true;
                        }
                        // Clear tab caches for source folders
                        for tab in self.tab_manager.tabs.iter_mut() {
                            let tab_path = normalize_for_match(Path::new(&tab.path));
                            if tab_path == source_str {
                                tab.items = std::sync::Arc::new(Vec::new());
                                tab.all_items.clear();
                            }
                        }
                    }

                    // Clear tab caches for destination
                    for tab in self.tab_manager.tabs.iter_mut() {
                        let tab_path = normalize_for_match(Path::new(&tab.path));
                        if tab_path == dest_str {
                            tab.items = std::sync::Arc::new(Vec::new());
                            tab.all_items.clear();
                        }
                    }

                    if should_reload {
                        self.loaded_path.clear();
                        self.load_folder(false);
                    }

                    // Destination logic
                    if current_str == dest_str {
                        eprintln!(
                            "[MOVE-BATCH] Dest folder matches current view, reloading: {}",
                            self.current_path
                        );
                        self.loaded_path.clear();
                        self.load_folder(false);
                    }
                }
                FileOperationResult::Finished => {
                    self.file_ops_in_progress = self.file_ops_in_progress.saturating_sub(1);
                    if self.file_ops_in_progress == 0 {
                        // Operations done — completion handlers already triggered reload,
                        // so discard any watcher-accumulated auto-reload to avoid double refresh
                        self.pending_auto_reload = false;
                        // NOTE: pending_deletions is NOT cleared here because Finished and
                        // DeleteCompleted are processed in the same loop iteration. The folder
                        // reload triggered by DeleteCompleted hasn't completed yet — clearing
                        // now would allow thumbnail re-extraction for the deleted file during
                        // the reload. Instead, pending_deletions is cleared when folder loading
                        // finishes (saw_end_of_load) or on user cancel (no active load).
                        if !self.is_loading_folder {
                            self.pending_deletions.clear();
                        }
                    }
                }
            }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            }
        }

        // 2. CHECK DE AUTO-REFRESH (WATCHER)
        fn clean_path(p: &Path) -> PathBuf {
            let s = p.to_string_lossy().to_string();
            if let Some(stripped) = s.strip_prefix(r"\\?\") {
                PathBuf::from(stripped)
            } else {
                p.to_path_buf()
            }
        }

        let current_path_norm = normalize_for_match(Path::new(&self.current_path));
        // PERFORMANCE: Filter by file name only - no filesystem I/O.
        // Hidden/system attribute filtering is already done in load_folder().
        // Previously called std::fs::metadata() here which caused synchronous
        // HDD reads on the UI thread for every watcher event.
        let should_ignore = |p: &Path| -> bool {
            let name = p
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase();
            name.starts_with("dumpstack.log")
                || name.starts_with("hiberfil.sys")
                || name.starts_with("pagefile.sys")
                || name.starts_with("swapfile.sys")
                || name == "desktop.ini"
                || name == "thumbs.db"
        };

        // Drive-wide watcher (File Pilot optimization)
        let drive_events = self.drive_watcher.poll_events();
        let drive_watcher_active = !drive_events.is_empty();
        for event in drive_events {
            
            match &event {
                crate::infrastructure::drive_watcher::DriveWatcherEvent::Created(path) => {
                    if should_ignore(path) { continue; }
                    if let Some(parent) = path.parent() {
                        let parent_norm = normalize_for_match(parent);
                        if parent_norm == current_path_norm {
                            eprintln!("[FS-WATCH] CREATE: {:?}", path.file_name().unwrap_or_default());
                            self.pending_auto_reload = true;
                        }
                    }
                }
                crate::infrastructure::drive_watcher::DriveWatcherEvent::Deleted(path) => {
                    if should_ignore(path) { continue; }
                    let cleaned = clean_path(path);
                    if let Some(parent) = cleaned.parent() {
                        self.directory_cache.invalidate(&parent.to_path_buf());
                    }
                    self.directory_cache.invalidate_children(&cleaned);
                    self.disk_cache.remove_cache_for_path(&cleaned);
                    
                    if let Some(parent) = path.parent() {
                        let parent_norm = normalize_for_match(parent);
                        if parent_norm == current_path_norm {
                            eprintln!("[FS-WATCH] DELETE: {:?}", path.file_name().unwrap_or_default());
                            
                            // SMART DELETE: Remove da UI sem reload completo
                            let path_to_remove = cleaned.clone();
                            let removed_from_all = self.all_items.iter()
                                .position(|item| item.path == path_to_remove)
                                .map(|idx| {
                                    self.all_items.remove(idx);
                                    true
                                })
                                .unwrap_or(false);
                            
                            if removed_from_all {
                                // Atualiza items (Arc) - recria sem o item deletado
                                let filtered: Vec<_> = self.items.iter()
                                    .filter(|item| item.path != path_to_remove)
                                    .cloned()
                                    .collect();
                                self.items = Arc::new(filtered);
                                self.total_items = self.items.len();
                                eprintln!("[FS-WATCH] SMART DELETE: Removed from UI without reload");
                                
                                // Ajusta seleção se necessário
                                if let Some(selected) = self.selected_item {
                                    if selected >= self.items.len() && !self.items.is_empty() {
                                        self.selected_item = Some(self.items.len() - 1);
                                    } else if self.items.is_empty() {
                                        self.selected_item = None;
                                        self.selected_file = None;
                                    }
                                }
                                
                                // Previne reload desnecessário - UI já foi atualizada
                                self.skip_next_auto_reload = true;
                            }
                            
                            // Não triggera auto-reload - UI já foi atualizada
                            // self.pending_auto_reload = true;
                        }
                    }
                }
                crate::infrastructure::drive_watcher::DriveWatcherEvent::Modified(path) => {
                    if should_ignore(path) { continue; }
                    let cleaned = clean_path(path);
                    self.cache_manager.texture_cache.pop(&cleaned);
                    self.cache_manager.failed_thumbnails.pop(&cleaned);
                    crate::workers::thumbnail::clear_failure_cache(&cleaned);
                    
                    if let Some(parent) = path.parent() {
                        let parent_norm = normalize_for_match(parent);
                        if parent_norm == current_path_norm {
                            eprintln!("[FS-WATCH] MODIFY: {:?}", path.file_name().unwrap_or_default());
                            self.pending_auto_reload = true;
                        }
                    }
                }
                crate::infrastructure::drive_watcher::DriveWatcherEvent::Renamed(old_path, new_path) => {
                    if !should_ignore(old_path) || !should_ignore(new_path) {
                        let cleaned_old = clean_path(old_path);
                        let cleaned_new = clean_path(new_path);
                        
                        // Invalidate caches for both paths
                        self.cache_manager.texture_cache.pop(&cleaned_old);
                        self.cache_manager.texture_cache.pop(&cleaned_new);
                        self.cache_manager.failed_thumbnails.pop(&cleaned_old);
                        self.cache_manager.failed_thumbnails.pop(&cleaned_new);
                        
                        if let Some(parent) = cleaned_old.parent() {
                            self.directory_cache.invalidate(&parent.to_path_buf());
                        }
                        if let Some(parent) = cleaned_new.parent() {
                            let parent_norm = normalize_for_match(parent);
                            if parent_norm == current_path_norm {
                                self.pending_auto_reload = true;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        
        // LEGACY: Processa eventos do notify-watcher (mantido para compatibilidade)
        // Se drive-watcher já detectou eventos, skip notify-watcher para evitar duplicados
        #[cfg(feature = "notify-watcher")]
        if !drive_watcher_active {
        while let Ok(event) = self.fs_event_receiver.try_recv() {
            match event {
                Ok(evt) => {
                    let mut meaningful_change = false;

                    if matches!(evt.kind, notify::EventKind::Remove(_)) {
                        for path in &evt.paths {
                            if should_ignore(path) {
                                continue;
                            }
                            meaningful_change = true;

                            let cleaned = clean_path(path);
                            if let Some(parent) = cleaned.parent() {
                                self.directory_cache.invalidate(&parent.to_path_buf());
                            }
                            self.directory_cache.invalidate_children(&cleaned);
                            eprintln!(
                                "[FS-WATCH-LEGACY] REMOVE: {:?}",
                                path.file_name().unwrap_or_default()
                            );
                            self.disk_cache.remove_cache_for_path(&cleaned);
                        }
                    }

                    for path in &evt.paths {
                        if should_ignore(path) {
                            continue;
                        }
                        meaningful_change = true;

                        if let Some(parent) = path.parent() {
                            let parent_norm = normalize_for_match(parent);
                            if parent_norm == current_path_norm {
                                let cleaned = clean_path(path);
                                if let Some(cache_parent) = cleaned.parent() {
                                    self.directory_cache.invalidate(&cache_parent.to_path_buf());
                                }
                                eprintln!(
                                    "[FS] Direct subfolder modified: {:?}",
                                    cleaned.file_name()
                                );
                                self.cache_manager.invalidate_folder_preview(&cleaned);
                            }
                        }

                        if let Some(parent) = path.parent() {
                            if let Some(grandparent) = parent.parent() {
                                let grandparent_norm = normalize_for_match(grandparent);
                                if grandparent_norm == current_path_norm {
                                    let cleaned_parent = clean_path(parent);
                                    if let Some(cache_parent) = cleaned_parent.parent() {
                                        self.directory_cache
                                            .invalidate(&cache_parent.to_path_buf());
                                    }
                                    eprintln!(
                                        "[FS] File in subfolder modified, invalidating: {:?}",
                                        cleaned_parent.file_name()
                                    );
                                    self.cache_manager
                                        .invalidate_folder_preview(&cleaned_parent);
                                }
                            }
                        }

                        let cleaned = clean_path(path);
                        self.cache_manager.texture_cache.pop(&cleaned);
                        self.cache_manager.failed_thumbnails.pop(&cleaned);
                        crate::workers::thumbnail::clear_failure_cache(&cleaned);
                    }

                    if meaningful_change {
                        self.pending_auto_reload = true;
                    }
                }
                Err(e) => eprintln!("Erro de watch: {:?}", e),
            }
        }
        } // Fecha o if !drive_watcher_active

        // Executa reload apenas quando debounce permitir
        // SUPPRESS auto-reload while file operations are in progress to prevent
        // screen flashing (watcher fires repeatedly as files grow during copy)
        // Skip auto-reload if smart delete already updated the UI
        if self.skip_next_auto_reload {
            self.skip_next_auto_reload = false;
            self.pending_auto_reload = false;
            eprintln!("[DEBUG] Skipping auto-reload - UI already updated by smart delete");
        }
        
        if self.pending_auto_reload && self.file_ops_in_progress == 0 {
            let elapsed = self.last_auto_reload.elapsed();
            if elapsed > Duration::from_millis(theme::AUTO_RELOAD_MS) {
                eprintln!(
                    "[DEBUG] Checking auto-reload for path: '{}'",
                    self.current_path
                );
                // SKIP for special views (Recycle Bin/Computer) which are managed manually via events
                if self.is_recycle_bin_view || self.is_computer_view {
                    self.pending_auto_reload = false;
                } else {
                    eprintln!("[DEBUG] Auto-reloading with force_refresh=false (watcher-triggered).");
                    // PERFORMANCE: Use force_refresh=false for watcher-triggered reloads.
                    // force_refresh=true clears ALL caches (textures, thumbnails, folder covers),
                    // empties the items list, and causes a white screen on HDD while rescanning.
                    // With false: directory_cache was already invalidated by watcher events above,
                    // so fresh data is loaded from disk, but texture/thumbnail caches are preserved.
                    // force_refresh=true is reserved for manual refresh (F5) only.
                    self.loaded_path.clear();
                    self.load_folder(false);
                }
                self.last_auto_reload = Instant::now();
                self.pending_auto_reload = false;
            }
        }

        // 1. STREAMING: Recebe lotes incrementais de FileEntry (Filtrado por geração)
        // BLOCKING: Process all available file entries in batch
        let mut saw_end_of_load = false;
        loop {
            match self.file_entry_receiver.try_recv() {
                Ok((gen_id, new_batch)) => {
                    if gen_id != self.generation {
                        continue; // Descarta dados de uma navegação/refresh anterior
                    }

                    if new_batch.is_empty() {
                        // Lote vazio = Sinal de "Fim do Carregamento" da thread
                        saw_end_of_load = true;
                    } else {
                        // Chegou dados! Adiciona à lista mestre
                        self.pending_items_count = self.pending_items_count.saturating_add(new_batch.len());
                        self.pending_items_rebuild = true;
                        self.all_items.extend(new_batch);
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break, // No more messages
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            }
        }

        if saw_end_of_load {
            self.is_loading_folder = false;
            self.pending_deletions.clear();
            self.pending_items_rebuild = false;
            self.pending_items_count = 0;
            // Ordenação final em background (evita stutter no UI thread)
            self.items_rebuild_request_id = self.items_rebuild_request_id.wrapping_add(1);
            let request_id = self.items_rebuild_request_id;
            let gen = self.generation;
            let items = self.all_items.clone();
            let query = self.search_query.clone();
            let sort_mode = self.sort_mode;
            let sort_descending = self.sort_descending;
            let folders_position = self.folders_position;
            let sender = self.items_rebuild_sender.clone();
            std::thread::spawn(move || {
                let mut result_items = match sorting::filter_items_opt(&items, &query) {
                    Some(filtered) => filtered,
                    None => {
                        let mut all = items;
                        sorting::sort_items(&mut all, sort_mode, sort_descending, folders_position);
                        all
                    }
                };
                if !query.is_empty() {
                    sorting::sort_items(&mut result_items, sort_mode, sort_descending, folders_position);
                }
                let total = result_items.len();
                let _ = sender.send(ItemsRebuildResult {
                    generation: gen,
                    request_id,
                    items: result_items,
                    total_items: total,
                });
            });
            self.last_items_rebuild = Instant::now();
            ctx.request_repaint();
        } else if self.pending_items_rebuild {
            // Throttle rebuild para evitar sort a cada lote
            let elapsed = self.last_items_rebuild.elapsed();
            if elapsed > Duration::from_millis(80) || self.pending_items_count >= 1200 {
                self.items_rebuild_request_id = self.items_rebuild_request_id.wrapping_add(1);
                let request_id = self.items_rebuild_request_id;
                let gen = self.generation;
                let items = self.all_items.clone();
                let query = self.search_query.clone();
                let sort_mode = self.sort_mode;
                let sort_descending = self.sort_descending;
                let folders_position = self.folders_position;
                let sender = self.items_rebuild_sender.clone();
                std::thread::spawn(move || {
                    let mut result_items = match sorting::filter_items_opt(&items, &query) {
                        Some(filtered) => filtered,
                        None => {
                            let mut all = items;
                            sorting::sort_items(&mut all, sort_mode, sort_descending, folders_position);
                            all
                        }
                    };
                    if !query.is_empty() {
                        sorting::sort_items(&mut result_items, sort_mode, sort_descending, folders_position);
                    }
                    let total = result_items.len();
                    let _ = sender.send(ItemsRebuildResult {
                        generation: gen,
                        request_id,
                        items: result_items,
                        total_items: total,
                    });
                });
                self.last_items_rebuild = Instant::now();
                self.pending_items_count = 0;
                self.pending_items_rebuild = false;
                ctx.request_repaint();
            }
        }

        // 2. Cover Worker: Recebe resultados de capas de folder
        let mut folder_updates = false;
        while let Ok((folder_path, cover_opt)) = self.cover_worker_receiver.try_recv() {
            if let Some(cover) = cover_opt {
                // Atualiza em all_items (fonte mutável)
                if let Some(item) = self.all_items.iter_mut().find(|i| i.path == folder_path) {
                    item.folder_cover = Some(cover.clone());
                    // PERFORMANCE: DB write moved to worker thread to avoid main thread stutter
                    folder_updates = true;

                    // Requisita thumbnail se necessário (Marcando como em carregamento para evitar loop)
                    if !self.cache_manager.has_thumbnail(&cover)
                        && self.cache_manager.start_loading(cover.clone())
                    {
                        self.request_thumbnail_load(cover, 256);
                    }
                }
            }
        }
        // Reconstrói items a partir de all_items se houve updates
        if folder_updates {
            self.filter_items();
            ctx.request_repaint();
        }

        // 3. Icon Worker: Recebe resultados de ícones assíncronos
        // PERFORMANCE: Throttle icon uploads - reduce when video is playing
        let max_icon_uploads = if self.is_video_playing_docked() { 2 } else { 5 };
        let mut icon_uploads = 0;
        while icon_uploads < max_icon_uploads {
            if let Ok((path, pixels, width, height)) = self.icon_res_receiver.try_recv() {
                self.loading_icons.remove(&path);

                // Skip texture creation if extraction failed (empty data)
                // Track failed icons to prevent infinite retry loops
                if pixels.is_empty() || width == 0 || height == 0 {
                    self.failed_icons.insert(path);
                    icon_uploads += 1;
                    continue;
                }

                // Carrega textura no cache de ícones
                // FIX: Cache key must match icon_loader.rs format (path + size)
                // Icon worker uses IconSize::Jumbo for high-quality icons
                let cache_key = format!("{}_Jumbo", path.to_string_lossy());
                if !self.item_icon_loader.icon_cache.contains(&cache_key) {
                    let texture = ctx.load_texture(
                        cache_key.clone(),
                        egui::ColorImage::from_rgba_unmultiplied(
                            [width as usize, height as usize],
                            &pixels,
                        ),
                        egui::TextureOptions::NEAREST,
                    );
                    self.item_icon_loader.icon_cache.put(cache_key, texture);
                }
                icon_uploads += 1;
            } else {
                break;
            }
        }
        if icon_uploads >= max_icon_uploads {
            ctx.request_repaint();
        }

        // 4. Metadata Worker: drena respostas mesmo sem thumbnails
        let mut metadata_updated = false;
        while let Ok((path, mtime, meta)) = self.metadata_res_receiver.try_recv() {
            self.metadata_loading.remove(&path);
            self.metadata_cache.put(path.clone(), (mtime, meta.clone()));

            if let Some(selected) = &self.selected_file {
                if selected.path == path {
                    self.selected_metadata = Some((path.clone(), meta));
                    metadata_updated = true;
                }
            }
        }
        if metadata_updated {
            ctx.request_repaint();
        }

        // 5. Individual thumbnails
        let mut received_any = false;

        // PERFORMANCE: Drain ALL pending thumbnails from worker into a persistent buffer
        // This ensures no data is lost when throttling GPU uploads.
        // PERFORMANCE: Limit pending_thumbnails buffer to prevent RAM spikes
        // Each thumbnail data can be ~1MB, so limit to ~100MB worth of pending data
        const MAX_PENDING_THUMBNAILS: usize = 100;
        
        while let Ok(thumbnail_data) = self.image_receiver.try_recv() {
            // Se a imagem pertence a uma geração anterior (outra folder), descarta.
            if thumbnail_data.generation != self.generation {
                continue;
            }

            // Sempre libera o slot de loading, mesmo em falhas
            self.cache_manager.finish_loading(&thumbnail_data.path);

            // Se a imagem veio vazia, marca como falha para evitar retry infinito
            if thumbnail_data.image_data.is_empty() {
                self.cache_manager
                    .mark_as_failed(thumbnail_data.path.clone());

                // Stale folder cover cleanup: file was deleted from disk
                // Remove stale DB entry and re-discover a new cover asynchronously
                if thumbnail_data.not_found {
                    let failed = &thumbnail_data.path;
                    for item in self.all_items.iter_mut() {
                        if item.folder_cover.as_ref() == Some(failed) {
                            let folder = item.path.clone();
                            item.folder_cover = None;
                            self.disk_cache.remove_folder_cover(&folder);
                            let _ = self.cover_worker_sender.send(folder);
                        }
                    }
                }

                continue;
            }

            // PERFORMANCE: Drop oldest thumbnails if buffer is full
            // This prevents RAM spikes when workers produce faster than GPU upload
            while self.pending_thumbnails.len() >= MAX_PENDING_THUMBNAILS {
                if let Some(old) = self.pending_thumbnails.pop_front() {
                    self.cache_manager.finish_pending_upload(&old.path);
                }
            }

            // Adiciona ao buffer persistente para upload posterior
            self.cache_manager
                .start_pending_upload(thumbnail_data.path.clone());
            self.pending_thumbnails.push_back(thumbnail_data);
            received_any = true;
        }

        // PERFORMANCE: Adaptive GPU upload throttling based on scroll state AND video playback
        // Note: Thumbnail cache is on SSD, so we can be more generous with uploads
        let is_scrolling = self.last_scroll_time.elapsed() < std::time::Duration::from_millis(100);
        let is_video_playing = self.is_video_playing_docked();

        // CRITICAL PERFORMANCE MODE: Skip all non-essential uploads when FPS is critically low
        // This prevents compounding performance issues during heavy load
        const CRITICAL_FRAME_TIME_MS: f32 = 33.33; // < 30 FPS
        const SEVERE_FRAME_TIME_MS: f32 = 25.0;    // < 40 FPS
        
        let is_performance_critical = self.frame_time_peak_ms > CRITICAL_FRAME_TIME_MS;
        let is_performance_severe = self.frame_time_peak_ms > SEVERE_FRAME_TIME_MS;

        let base_max_uploads = if is_performance_critical {
            1 // Minimal: only most essential uploads
        } else if is_performance_severe {
            2 // Reduced: critical performance mode
        } else if is_video_playing && is_scrolling {
            4 // Balanced: still load during scroll+video
        } else if is_scrolling {
            6 // Generous during scroll — time budget is the real limiter
        } else if is_video_playing {
            5 // Moderate limit during video
        } else {
            12 // Aggressive idle speed — fill visible area fast
        };
        let perf_scale = if self.frame_time_avg_ms <= 0.0 {
            1.0
        } else if self.frame_time_avg_ms < 12.0 {
            1.25
        } else if self.frame_time_avg_ms < 18.0 {
            1.0
        } else if self.frame_time_avg_ms < 24.0 {
            0.85
        } else {
            0.7
        };
        let max_uploads_per_frame = ((base_max_uploads as f32) * perf_scale)
            .round()
            .clamp(1.0, 16.0) as usize;

        let mut uploads_this_frame = 0;
        let upload_start = Instant::now();
        let now = Instant::now();
        if now.duration_since(self.last_upload_budget_update) > Duration::from_millis(750) {
            let target_budget_ms = if self.frame_time_avg_ms <= 0.0 {
                self.upload_budget_ms
            } else if self.frame_time_avg_ms < 12.0 {
                8.0
            } else if self.frame_time_avg_ms < 18.0 {
                6.0
            } else if self.frame_time_avg_ms < 24.0 {
                4.0
            } else {
                3.0
            };
            if (self.upload_budget_ms - target_budget_ms).abs() >= 0.5 {
                self.upload_budget_ms = target_budget_ms.clamp(2.0, 10.0);
                self.disk_cache
                    .set_preference("upload_budget_ms", &self.upload_budget_ms.to_string());
            }
            self.last_upload_budget_update = now;
        }

        let base_budget_ms = if is_video_playing && is_scrolling {
            self.upload_budget_ms * 0.6
        } else if is_video_playing {
            self.upload_budget_ms * 0.75
        } else if is_scrolling {
            self.upload_budget_ms * 0.85
        } else {
            self.upload_budget_ms
        };
        let upload_budget_ms = (base_budget_ms * perf_scale).clamp(2.0, 10.0);
        let upload_budget = Duration::from_millis(upload_budget_ms.round() as u64);

        // PERFORMANCE: Build set of visible item paths for upload prioritization
        // During scroll, visible items get uploaded first; off-screen items are deferred
        let visible_paths: Option<crate::ui::cache::FxHashSet<PathBuf>> = if is_scrolling {
            self.visible_index_range.and_then(|(min_idx, max_idx)| {
                let items = &self.items;
                if items.is_empty() {
                    return None;
                }
                let max_idx = max_idx.min(items.len().saturating_sub(1));
                Some(
                    (min_idx..=max_idx)
                        .map(|i| items[i].path.clone())
                        .collect(),
                )
            })
        } else {
            None
        };
        let mut deferred_count = 0;

        // Process thumbnails from the buffer up to the per-frame limit
        while uploads_this_frame < max_uploads_per_frame {
            if let Some(thumbnail_data) = self.pending_thumbnails.pop_front() {
                if upload_start.elapsed() >= upload_budget {
                    self.pending_thumbnails.push_front(thumbnail_data);
                    break;
                }
                // Ensure thumbnail is still relevant (generation check again just in case)
                if thumbnail_data.generation != self.generation {
                    self.cache_manager
                        .finish_pending_upload(&thumbnail_data.path);
                    continue;
                }

                // PERFORMANCE: In critical mode, only process visible items
                // Skip non-visible uploads entirely to maintain responsiveness
                if is_performance_critical {
                    if let Some(ref vis) = visible_paths {
                        if !vis.contains(&thumbnail_data.path) {
                            // Defer to back of queue - will retry later when performance recovers
                            self.pending_thumbnails.push_back(thumbnail_data);
                            deferred_count += 1;
                            if deferred_count > max_uploads_per_frame * 2 {
                                break;
                            }
                            continue;
                        }
                    }
                }

                // PERFORMANCE: During scroll, prioritize visible items
                // Off-screen thumbnails are deferred to the back of the queue
                if let Some(ref vis) = visible_paths {
                    if !vis.contains(&thumbnail_data.path) {
                        self.pending_thumbnails.push_back(thumbnail_data);
                        deferred_count += 1;
                        // Safety limit: don't loop through entire queue
                        if deferred_count > max_uploads_per_frame * 3 {
                            break;
                        }
                        continue;
                    }
                }

                // PERFORMANCE: Store RGBA data in RAM cache before GPU upload
                // This allows fast re-upload if texture is evicted from VRAM without disk I/O
                let path = thumbnail_data.path.clone();
                let width = thumbnail_data.width;
                let height = thumbnail_data.height;
                self.cache_manager.put_rgba_data(
                    path.clone(),
                    thumbnail_data.image_data,
                    width,
                    height,
                );

                // Carrega textura no GPU
                let texture =
                    if let Some((rgba_data, _, _)) = self.cache_manager.get_rgba_data(&path) {
                        ctx.load_texture(
                            path.to_string_lossy().to_string(),
                            egui::ColorImage::from_rgba_unmultiplied(
                                [width as usize, height as usize],
                                rgba_data,
                            ),
                            egui::TextureOptions::NEAREST,
                        )
                    } else {
                        self.cache_manager.finish_pending_upload(&path);
                        continue;
                    };

                self.cache_manager
                    .put_thumbnail(path.clone(), texture.clone());

                // Limpa status de pending upload
                self.cache_manager.finish_pending_upload(&path);

                // Update selected_thumbnail if it matches the selected_file
                if let Some(selected_file) = &self.selected_file {
                    if selected_file.path == path {
                        self.selected_thumbnail = Some(texture);
                    }
                }

                uploads_this_frame += 1;
                received_any = true;
            } else {
                break; // Buffer is empty
            }
        }

        // PERFORMANCE: Single repaint request after upload loop (not per-upload)
        if !self.pending_thumbnails.is_empty() {
            ctx.request_repaint();
        }

        // 6. Folder Previews (Native Sandwich effect)
        // PERFORMANCE: Throttle folder preview uploads (Max 2 per frame - heavy textures)
        let mut folder_uploads = 0;
        while folder_uploads < 2 {
            if let Ok(data) = self.folder_preview_receiver.try_recv() {
                self.cache_manager.finish_folder_preview_loading(&data.path);

                // Only create texture if we have actual data
                if !data.rgba_data.is_empty() {
                    let texture = ctx.load_texture(
                        format!("folder_preview_{}", data.path.to_string_lossy()),
                        egui::ColorImage::from_rgba_unmultiplied(
                            [data.width as usize, data.height as usize],
                            &data.rgba_data,
                        ),
                        egui::TextureOptions::NEAREST,
                    );

                    self.cache_manager.put_folder_preview(data.path, texture);
                }
                folder_uploads += 1;
            } else {
                break;
            }
        }
        if folder_uploads >= 2 {
            ctx.request_repaint();
        }

        // 9. FOLDER SIZE RESULTS
        while let Ok((folder_path, total_size)) = self.folder_size_res_receiver.try_recv() {
            self.folder_size_loading.remove(&folder_path);
            self.folder_size_cache.insert(folder_path, total_size);
            received_any = true;
        }

        if received_any {
            ctx.request_repaint();
        }
    }
}
