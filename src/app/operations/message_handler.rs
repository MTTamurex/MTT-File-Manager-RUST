//! Async message processing from workers
//!
//! This module processes incoming messages from various background workers
//! (filesystem events, thumbnails, folder sizes, etc.) and updates the UI state.

use std::time::{Duration, Instant};
use std::path::{Path, PathBuf};
use eframe::egui;
use crate::app::state::ImageViewerApp;
use crate::ui::theme;

impl ImageViewerApp {
    pub fn process_incoming_messages(&mut self, ctx: &egui::Context) {
        // 1. CHECK DE REFRESH MANUAL (F5)
        if ctx.input(|i| i.key_pressed(egui::Key::F5)) {
            self.trigger_manual_refresh();
        }

        while self.device_event_receiver.try_recv().is_ok() {
            if self.reload_drive_list() {
                self.last_drive_refresh = Instant::now();
                if self.is_computer_view {
                    self.setup_computer_view();
                }
                // Force immediate repaint without waiting for input events
                ctx.request_repaint_after(std::time::Duration::from_millis(0));
            }
        }

        // 2. CHECK DE AUTO-REFRESH (WATCHER)
        fn normalize_for_match(p: &Path) -> String {
            let s = p.to_string_lossy().to_string().to_lowercase();
            if let Some(stripped) = s.strip_prefix(r"\\?\") {
                stripped.to_string()
            } else {
                s
            }
        }

        fn clean_path(p: &Path) -> PathBuf {
            let s = p.to_string_lossy().to_string();
            if let Some(stripped) = s.strip_prefix(r"\\?\") {
                PathBuf::from(stripped)
            } else {
                p.to_path_buf()
            }
        }

        let current_path_norm = normalize_for_match(Path::new(&self.current_path));

        while let Ok(event) = self.fs_event_receiver.try_recv() {
            match event {
                Ok(evt) => {
                    let mut meaningful_change = false;

                    // Filter out hidden/system files to prevent infinite reload loops (e.g. C:\DumpStack.log.tmp)
                    let should_ignore = |p: &Path| -> bool {
                        let name = p
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_lowercase();
                        // Ignore common noisy system files
                        if name.starts_with("dumpstack.log")
                            || name.starts_with("hiberfil.sys")
                            || name.starts_with("pagefile.sys")
                            || name.starts_with("swapfile.sys")
                            || name == "desktop.ini"
                            || name == "thumbs.db"
                        {
                            return true;
                        }

                        // Check attributes if file exists
                        if let Ok(metadata) = std::fs::metadata(p) {
                            // 0x02 = Hidden, 0x04 = System
                            use std::os::windows::fs::MetadataExt;
                            let attrs = metadata.file_attributes();
                            if (attrs & 0x02) != 0 || (attrs & 0x04) != 0 {
                                return true;
                            }
                        }
                        false
                    };

                    // Detecta eventos de Remove para limpar cache automaticamente
                    if matches!(evt.kind, notify::EventKind::Remove(_)) {
                        for path in &evt.paths {
                            if should_ignore(path) {
                                continue;
                            }
                            meaningful_change = true;

                            let cleaned = clean_path(path);
                            eprintln!(
                                "[FS] Detected removal, clearing disk cache for: {:?}",
                                cleaned
                            );
                            self.disk_cache.remove_cache_for_path(&cleaned);
                        }
                    }

                    // Detecta Modify para invalidar folder previews
                    for path in &evt.paths {
                        if should_ignore(path) {
                            continue;
                        }
                        meaningful_change = true;

                        // 1. Se o path alterado é uma subpasta direta da pasta atual
                        if let Some(parent) = path.parent() {
                            let parent_norm = normalize_for_match(parent);
                            if parent_norm == current_path_norm {
                                let cleaned = clean_path(path);
                                eprintln!(
                                    "[FS] Direct subfolder modified: {:?}",
                                    cleaned.file_name()
                                );
                                self.cache_manager.invalidate_folder_preview(&cleaned);
                            }
                        }

                        // 2. Se o path alterado é UM ARQUIVO dentro de uma subpasta da pasta atual
                        if let Some(parent) = path.parent() {
                            if let Some(grandparent) = parent.parent() {
                                let grandparent_norm = normalize_for_match(grandparent);
                                if grandparent_norm == current_path_norm {
                                    let cleaned_parent = clean_path(parent);
                                    eprintln!(
                                        "[FS] File in subfolder modified, invalidating: {:?}",
                                        cleaned_parent.file_name()
                                    );
                                    self.cache_manager
                                        .invalidate_folder_preview(&cleaned_parent);
                                }
                            }
                        }
                    }

                    if meaningful_change {
                        self.pending_auto_reload = true;
                    }
                }
                Err(e) => eprintln!("Erro de watch: {:?}", e),
            }
        }

        // Executa reload apenas quando debounce permitir
        if self.pending_auto_reload {
            let elapsed = self.last_auto_reload.elapsed();
            if elapsed > Duration::from_millis(theme::AUTO_RELOAD_MS) {
                // VALIDA SE O PATH ATUAL AINDA EXISTE (pode ter sido renomeado/deletado)
                if Path::new(&self.current_path).exists() {
                    self.load_folder(true); // force_refresh para atualizar thumbnails modificados
                } else {
                    self.go_up_one_level();
                }
                self.last_auto_reload = Instant::now();
                self.pending_auto_reload = false;
            }
        }

        // 1. STREAMING: Recebe lotes incrementais de FileEntry (Filtrado por geração)
        while let Ok((gen_id, new_batch)) = self.file_entry_receiver.try_recv() {
            if gen_id != self.generation {
                continue; // Descarta dados de uma navegação/refresh anterior
            }

            if new_batch.is_empty() {
                // Lote vazio = Sinal de "Fim do Carregamento" da thread
                self.is_loading_folder = false;
                // Ordenação final para garantir tudo correto
                self.sort_items();
            } else {
                // Chegou dados! Adiciona à lista mestre
                self.all_items.extend(new_batch);

                // Reaplica filtro (que já chama sort_items internamente)
                self.filter_items();
            }
            ctx.request_repaint();
        }

        // 2. Cover Worker: Recebe resultados de capas de folder
        let mut folder_updates = false;
        while let Ok((folder_path, cover_opt)) = self.cover_worker_receiver.try_recv() {
            if let Some(cover) = cover_opt {
                // Atualiza em all_items (fonte mutável)
                if let Some(item) = self.all_items.iter_mut().find(|i| i.path == folder_path) {
                    item.folder_cover = Some(cover.clone());
                    self.disk_cache.set_folder_cover(&folder_path, &cover);
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
        while let Ok((path, pixels, width, height)) = self.icon_res_receiver.try_recv() {
            self.loading_icons.remove(&path);

            // Carrega textura no cache de ícones
            let cache_key = path.to_string_lossy().to_string();
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
        let mut _new_items_added = false;

        while let Ok(thumbnail_data) = self.image_receiver.try_recv() {
            // --- VALIDAÇÃO DE MEMÓRIA ---
            // Se a imagem pertence a uma geração anterior (outra folder), descarta.
            if thumbnail_data.generation != self.generation {
                continue;
            }
            // ----------------------------

            received_any = true;

            // Só processa thumbnails (image_data não vazio)
            if !thumbnail_data.image_data.is_empty() {
                self.cache_manager.finish_loading(&thumbnail_data.path);

                let texture = ctx.load_texture(
                    thumbnail_data.path.to_string_lossy().to_string(),
                    egui::ColorImage::from_rgba_unmultiplied(
                        [
                            thumbnail_data.width as usize,
                            thumbnail_data.height as usize,
                        ],
                        &thumbnail_data.image_data,
                    ),
                    egui::TextureOptions::NEAREST,
                );

                self.cache_manager
                    .put_thumbnail(thumbnail_data.path.clone(), texture.clone());

                // Update selected_thumbnail if it matches the selected_file
                if let Some(selected_file) = &self.selected_file {
                    if selected_file.path == thumbnail_data.path {
                        self.selected_thumbnail = Some(texture);
                    }
                }
            }
        }

        // 6. Folder Previews (Native Sandwich effect)
        while let Ok(data) = self.folder_preview_receiver.try_recv() {
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
