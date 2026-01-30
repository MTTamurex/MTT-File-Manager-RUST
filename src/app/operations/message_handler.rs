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
        while let Ok(result) = self.items_rebuild_receiver.try_recv() {
            if result.generation != self.generation {
                continue;
            }
            if result.request_id != self.items_rebuild_request_id {
                continue;
            }
            self.items = Arc::new(result.items);
            self.total_items = result.total_items;
            ctx.request_repaint();
        }

        fn normalize_for_match(p: &Path) -> String {
            let s = p.to_string_lossy().to_string().to_lowercase();
            if let Some(stripped) = s.strip_prefix(r"\\?\") {
                stripped.to_string()
            } else {
                s
            }
        }

        while let Ok(res) = self.file_op_res_receiver.try_recv() {
            use crate::workers::file_operation_worker::FileOperationResult;
            match res {
                FileOperationResult::RenameCompleted {
                    path,
                    new_name,
                    parent_folder,
                } => {
                    let current_str = normalize_for_match(Path::new(&self.current_path));
                    let parent_str = normalize_for_match(parent_folder.as_path());
                    self.directory_cache.invalidate(&parent_folder);
                    if let Some(di) = &self.directory_index {
                        let _ = di.invalidate(&parent_folder);
                    }

                    // FIX: If the renamed item is currently selected, update the selection state
                    // This prevents stale data in the Details Panel even before reload completes.
                    // Note: load_folder() does NOT clear selected_file, so this persists correctly.
                    if let Some(selected) = &mut self.selected_file {
                        if normalize_for_match(&selected.path) == normalize_for_match(&path) {
                            let new_path = parent_folder.join(&new_name);
                            selected.path = new_path;
                            selected.name = new_name;
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

                    if dest_str == current_str {
                        eprintln!(
                            "[COPY] Dest folder matches current view, reloading: {}",
                            self.current_path
                        );
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

                    if current_str == source_str {
                        eprintln!(
                            "[MOVE] Source folder matches current view, reloading: {}",
                            self.current_path
                        );
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
                        self.load_folder(false);
                    }
                }
                FileOperationResult::Finished => {}
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
        let should_ignore = |p: &Path| -> bool {
            let name = p
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase();
            if name.starts_with("dumpstack.log")
                || name.starts_with("hiberfil.sys")
                || name.starts_with("pagefile.sys")
                || name.starts_with("swapfile.sys")
                || name == "desktop.ini"
                || name == "thumbs.db"
            {
                return true;
            }

            if let Ok(metadata) = std::fs::metadata(p) {
                use std::os::windows::fs::MetadataExt;
                let attrs = metadata.file_attributes();
                if (attrs & 0x02) != 0 || (attrs & 0x04) != 0 {
                    return true;
                }
            }
            false
        };

        #[cfg(feature = "notify-watcher")]
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
                                "[FS] Detected removal, clearing disk cache for: {:?}",
                                cleaned
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
                        crate::workers::thumbnail_worker::clear_failure_cache(&cleaned);
                    }

                    if meaningful_change {
                        self.pending_auto_reload = true;
                    }
                }
                Err(e) => eprintln!("Erro de watch: {:?}", e),
            }
        }

        #[cfg(feature = "usn-watcher")]
        while let Ok(event) = self.fs_event_receiver.try_recv() {
            let mut meaningful_change_local = false;

            let handle_remove = |path: &Path| -> bool {
                if should_ignore(path) {
                    return false;
                }
                let cleaned = clean_path(path);
                if let Some(parent) = cleaned.parent() {
                    self.directory_cache.invalidate(&parent.to_path_buf());
                }
                self.directory_cache.invalidate_children(&cleaned);
                eprintln!(
                    "[FS] Detected removal, clearing disk cache for: {:?}",
                    cleaned
                );
                self.disk_cache.remove_cache_for_path(&cleaned);
                true
            };

            let mut handle_modify = |path: &Path| -> bool {
                if should_ignore(path) {
                    return false;
                }
                if let Some(parent) = path.parent() {
                    let parent_norm = normalize_for_match(parent);
                    if parent_norm == current_path_norm {
                        let cleaned = clean_path(path);
                        if let Some(cache_parent) = cleaned.parent() {
                            self.directory_cache.invalidate(&cache_parent.to_path_buf());
                        }
                        eprintln!("[FS] Direct subfolder modified: {:?}", cleaned.file_name());
                        self.cache_manager.invalidate_folder_preview(&cleaned);
                        self.disk_cache.remove_folder_cover(&cleaned);
                    }
                }

                if let Some(parent) = path.parent() {
                    if let Some(grandparent) = parent.parent() {
                        let grandparent_norm = normalize_for_match(grandparent);
                        if grandparent_norm == current_path_norm {
                            let cleaned_parent = clean_path(parent);
                            if let Some(cache_parent) = cleaned_parent.parent() {
                                self.directory_cache.invalidate(&cache_parent.to_path_buf());
                            }
                            eprintln!(
                                "[FS] File in subfolder modified, invalidating: {:?}",
                                cleaned_parent.file_name()
                            );
                            self.cache_manager
                                .invalidate_folder_preview(&cleaned_parent);
                            self.disk_cache.remove_folder_cover(&cleaned_parent);
                        }
                    }
                }

                let cleaned = clean_path(path);
                self.cache_manager.texture_cache.pop(&cleaned);
                self.cache_manager.failed_thumbnails.pop(&cleaned);
                crate::workers::thumbnail_worker::clear_failure_cache(&cleaned);
                true
            };

            match event {
                crate::workers::usn_watcher::FsEvent::Created(path) => {
                    if handle_modify(&path) {
                        meaningful_change_local = true;
                    }
                }
                crate::workers::usn_watcher::FsEvent::Deleted(path) => {
                    if handle_remove(&path) {
                        meaningful_change_local = true;
                    }
                    if handle_modify(&path) {
                        meaningful_change_local = true;
                    }
                }
                crate::workers::usn_watcher::FsEvent::Modified(path) => {
                    if handle_modify(&path) {
                        meaningful_change_local = true;
                    }
                }
                crate::workers::usn_watcher::FsEvent::Renamed(old_path, new_path) => {
                    if handle_remove(&old_path) {
                        meaningful_change_local = true;
                    }
                    if handle_modify(&new_path) {
                        meaningful_change_local = true;
                    }
                }
            }

            if meaningful_change_local {
                self.pending_auto_reload = true;
            }
        }

        // Executa reload apenas quando debounce permitir
        if self.pending_auto_reload {
            let elapsed = self.last_auto_reload.elapsed();
            if elapsed > Duration::from_millis(theme::AUTO_RELOAD_MS) {
                eprintln!(
                    "[DEBUG] Checking auto-reload for path: '{}'",
                    self.current_path
                );
                // VALIDA SE O PATH ATUAL AINDA EXISTE (pode ter sido renomeado/deletado)
                // SKIP for special views (Recycle Bin/Computer) which are managed manually via events
                if self.is_recycle_bin_view || self.is_computer_view {
                    self.pending_auto_reload = false;
                } else if Path::new(&self.current_path).exists() {
                    eprintln!("[DEBUG] Path exists. Reloading.");
                    self.load_folder(false); // false = don't clear entire cache, already cleared specific changed items
                } else {
                    eprintln!("[DEBUG] Path DOES NOT EXIST! Triggering go_up_one_level");
                    self.go_up_one_level();
                }
                self.last_auto_reload = Instant::now();
                self.pending_auto_reload = false;
            }
        }

        // 1. STREAMING: Recebe lotes incrementais de FileEntry (Filtrado por geração)
        let mut saw_end_of_load = false;
        while let Ok((gen_id, new_batch)) = self.file_entry_receiver.try_recv() {
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

        if saw_end_of_load {
            self.is_loading_folder = false;
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
                continue;
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

        let base_max_uploads = if is_video_playing && is_scrolling {
            2 // Restore loading during scroll+video (balanced)
        } else if is_scrolling {
            3 // Moderate limit during scroll
        } else if is_video_playing {
            3 // Moderate limit during video
        } else {
            6 // Standard idle speed
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
            .clamp(1.0, 10.0) as usize;

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

                // If we still have more thumbnails in buffer, request another frame to keep processing
                if !self.pending_thumbnails.is_empty() {
                    ctx.request_repaint();
                }
            } else {
                break; // Buffer is empty
            }
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
