//! Folder loading: load_folder, filter_items, sort_items, refresh
//!
//! This module handles scanning folders, filtering results, sorting, and manual refresh triggers.

use std::path::PathBuf;
use std::sync::atomic::Ordering as AtomicOrdering;
use std::sync::Arc;
use std::time::Instant;

use std::os::windows::ffi::OsStringExt;
use windows::core::PCWSTR;
use windows::Win32::Storage::FileSystem::*;

use crate::app::state::ImageViewerApp;
use crate::application::sorting;
use crate::domain::file_entry::{FileEntry, SyncStatus};
use crate::infrastructure::adaptive_batch::{AdaptiveBatchConfig, AdaptiveBatchTracker};
use crate::infrastructure::directory_index::IndexedFile;
use crate::infrastructure::io_priority;
use crate::infrastructure::ntfs_reader;
use crate::infrastructure::onedrive;
use crate::infrastructure::windows::{is_shell_navigation_path, list_shell_folder};
use crate::workers::idle_warmup::IdleWarmupMessage;
use crate::workers::predictive_prefetch::PredictiveMessage;
use crate::workers::prefetch_worker::PrefetchMessage;

impl ImageViewerApp {
    /// Filtra e ordena itens baseado na query de busca atual.
    ///
    /// PERFORMANCE: Usa filter_items_opt() que evita clone quando query está vazia.
    /// Isso elimina alocações desnecessárias em 99% dos casos de uso.
    pub fn filter_items(&mut self) {
        // PERFORMANCE: filter_items_opt returns None when query is empty,
        // signaling we should use all_items directly without cloning.
        match sorting::filter_items_opt(&self.all_items, &self.search_query) {
            Some(filtered) => {
                // Query presente: usa o vetor filtrado
                self.items = Arc::new(filtered);
            }
            None => {
                // Query vazia: ordena all_items in-place e usa diretamente
                // Isso evita um clone completo de todo o vetor
                sorting::sort_items(
                    &mut self.all_items,
                    self.sort_mode,
                    self.sort_descending,
                    self.folders_position,
                );
                self.items = Arc::new(self.all_items.clone());
            }
        }
        self.total_items = self.items.len();

        // Se houve filtragem, ainda precisamos ordenar o resultado
        if !self.search_query.is_empty() {
            self.sort_items();
        }
    }

    /// Ordena itens baseado no modo atual e preferência de posição de pastas.
    ///
    /// OTIMIZADO:
    /// - Usa par_sort_by para listas >5000 itens (rayon)
    /// - Usa comparações case-insensitive sem alocação (natord::compare_ignore_case)
    pub fn sort_items(&mut self) {
        // PERFORMANCE: Se temos ownership único do Arc, podemos modificar in-place
        // usando Arc::make_mut(). Caso contrário, precisamos clonar.
        let items = Arc::make_mut(&mut self.items);
        sorting::sort_items(
            items,
            self.sort_mode,
            self.sort_descending,
            self.folders_position,
        );
    }

    /// Requisita scan assíncrono de uma pasta para descobrir primeira imagem.
    /// OTIMIZADO: Envia mensagem para worker único (zero overhead de threads)
    pub fn request_folder_scan(&self, folder_path: PathBuf) {
        // Apenas envia para fila - worker processa em background
        let _ = self.cover_worker_sender.send(folder_path);
    }

    pub fn load_folder(&mut self, force_refresh: bool) {
        self.generation += 1; // Incrementa a geração local
        self.current_generation
            .store(self.generation, AtomicOrdering::Relaxed); // Sincroniza com workers

        let current_path_buf = PathBuf::from(&self.current_path);
        let _ = self
            .predictive_sender
            .send(PredictiveMessage::NavigatedTo(current_path_buf.clone()));
        let history_paths: Vec<PathBuf> = self
            .navigation
            .paths
            .iter()
            .rev()
            .take(5)
            .filter(|p| p.len() >= 2 && p.chars().nth(1) == Some(':'))
            .map(PathBuf::from)
            .collect();
        if !history_paths.is_empty() {
            let _ = self
                .predictive_sender
                .send(PredictiveMessage::HistoryUpdated(history_paths));
        }
        let _ = self
            .idle_warmup_sender
            .send(IdleWarmupMessage::CurrentDirectory(current_path_buf));

        // 1. Limpeza de Estado (UI Thread)
        if force_refresh {
            self.cache_manager.texture_cache.clear();
            self.cache_manager.folder_preview_cache.clear();
            self.cache_manager.failed_thumbnails.clear();
            crate::workers::thumbnail_worker::clear_all_failures();
            self.directory_cache.clear();
        }

        self.items = Arc::new(Vec::new()); // Novo Arc vazio (antigo é dropped automaticamente)
        self.all_items.clear(); // Limpa backup mestre também
        self.cache_manager.loading_set.clear(); // Limpa apenas requisições pendentes, mantém cache de texturas
        self.cache_manager.folder_preview_loading.clear(); // Limpa folder preview loading
        self.cache_manager.pending_upload_set.clear(); // Limpa thumbnails aguardando upload GPU
        self.pending_thumbnails.clear(); // Limpa buffer de thumbnails pendentes
        self.loading_icons.clear(); // Limpa icon loading requests
        self.scanned_folders.clear();
        self.selected_item = None;
        self.is_loading_folder = true;
        self.total_items = 0;
        self.pending_items_rebuild = false;
        self.pending_items_count = 0;
        self.last_items_rebuild = Instant::now();

        let my_gen = self.generation;
        let gen_clone = self.current_generation.clone();
        let current_path = self.current_path.clone();
        let file_entry_sender = self.file_entry_sender.clone();
        let ctx = self.ui_ctx.clone();
        let disk_cache = self.disk_cache.clone();
        let directory_cache = self.directory_cache.clone();
        let directory_index_opt = self.directory_index.clone();
        let prefetch_sender = self.prefetch_sender.clone();
        let force_refresh = force_refresh;

        // STREAMING BATCH LOADING: Adaptive batch size based on disk type
        std::thread::spawn(move || {
            let scan_start = std::time::Instant::now();

            let base_path = if current_path.len() == 2 && current_path.ends_with(':') {
                format!("{}\\", current_path)
            } else {
                current_path.clone()
            };

            let is_ssd = io_priority::is_ssd(&PathBuf::from(&current_path));
            let config = AdaptiveBatchConfig {
                is_ssd,
                total_items: directory_index_opt
                    .as_ref()
                    .and_then(|di| di.get_directory(&PathBuf::from(&base_path)))
                    .map(|(meta, _)| meta.file_count),
            };
            let mut batch_tracker = AdaptiveBatchTracker::new(config);
            let mut batch_size = batch_tracker.batch_size();
            eprintln!(
                "[PERF] Starting folder scan: {:?} (batch_size={}, is_ssd={})",
                current_path, batch_size, is_ssd
            );

            let mut batch = Vec::with_capacity(batch_size);
            let mut all_entries_disk: Vec<FileEntry> = Vec::new();
            let mut batch_start = std::time::Instant::now();

            // Check if we are navigating a virtual Shell folder (like a ZIP)
            if is_shell_navigation_path(&PathBuf::from(&base_path), false) {
                if let Ok(shell_items) = list_shell_folder(&PathBuf::from(&base_path)) {
                    if gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
                        let _ = file_entry_sender.send((my_gen, shell_items));
                        let _ = file_entry_sender.send((my_gen, Vec::new()));
                        ctx.request_repaint();
                        return;
                    }
                }
            }

            if !force_refresh {
                if let Some(di) = &directory_index_opt {
                    let base = PathBuf::from(&base_path);
                    if !di.might_have_changed(&base) {
                        if let Some((_meta, indexed_files)) = di.get_directory(&base) {
                            let mut entries: Vec<FileEntry> = indexed_files
                                .into_iter()
                                .filter(|f| !f.name.starts_with('.'))
                                .map(|f| FileEntry {
                                    path: base.join(&f.name),
                                    name: f.name,
                                    is_dir: f.is_dir,
                                    size: if f.is_dir { 0 } else { f.size },
                                    modified: f.modified,
                                    folder_cover: None,
                                    drive_info: None,
                                    sync_status: SyncStatus::None,
                                    deletion_date: None,
                                    recycle_original_path: None,
                                })
                                .collect();

                            let folders: Vec<PathBuf> = entries
                                .iter()
                                .filter(|e| e.is_dir)
                                .map(|e| e.path.clone())
                                .collect();
                            if !folders.is_empty() {
                                let covers = disk_cache.get_folder_covers(&folders);
                                for entry in entries.iter_mut() {
                                    if entry.is_dir {
                                        if let Some(cover) = covers.get(&entry.path) {
                                            entry.folder_cover = Some(cover.clone());
                                        }
                                    }
                                }
                            }

                            directory_cache.put(base.clone(), entries.clone());

                            let mut offset = 0;
                            while offset < entries.len() {
                                if gen_clone.load(AtomicOrdering::Relaxed) != my_gen {
                                    return;
                                }
                                let end = (offset + batch_size).min(entries.len());
                                let chunk = entries[offset..end].to_vec();
                                let _ = file_entry_sender.send((my_gen, chunk));
                                ctx.request_repaint();
                                batch_tracker.record_batch(batch_start.elapsed(), end - offset);
                                batch_size = batch_tracker.batch_size();
                                batch_start = std::time::Instant::now();
                                offset = end;
                            }
                            let _ = file_entry_sender.send((my_gen, Vec::new()));
                            ctx.request_repaint();
                            return;
                        }
                    }
                }
            }

            if !force_refresh {
                if let Some(cached_entries) = directory_cache.get(&PathBuf::from(&base_path)) {
                    let mut offset = 0;
                    while offset < cached_entries.len() {
                        if gen_clone.load(AtomicOrdering::Relaxed) != my_gen {
                            return;
                        }
                        let end = (offset + batch_size).min(cached_entries.len());
                        let chunk = cached_entries[offset..end].to_vec();
                        let _ = file_entry_sender.send((my_gen, chunk));
                        ctx.request_repaint();
                        batch_tracker.record_batch(batch_start.elapsed(), end - offset);
                        batch_size = batch_tracker.batch_size();
                        batch_start = std::time::Instant::now();
                        offset = end;
                    }
                    let _ = file_entry_sender.send((my_gen, Vec::new()));
                    ctx.request_repaint();

                    if !is_ssd && gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
                        let subdirs: Vec<PathBuf> = cached_entries
                            .iter()
                            .filter(|e| e.is_dir)
                            .take(5)
                            .map(|e| e.path.clone())
                            .collect();
                        if !subdirs.is_empty() {
                            let _ = prefetch_sender.send(PrefetchMessage::Prefetch(subdirs));
                        }
                    }
                    return;
                }
            }

            // OPTIMIZATION: Use NtQueryDirectoryFile on HDD when available
            let use_fast_reader = !is_ssd && ntfs_reader::is_available();

            if use_fast_reader {
                if let Some(entries) = ntfs_reader::read_directory_fast(&PathBuf::from(&base_path))
                {
                    for dir_entry in entries {
                        if gen_clone.load(AtomicOrdering::Relaxed) != my_gen {
                            break;
                        }
                        let is_hidden = (dir_entry.attributes & 0x02) != 0;
                        let is_system = (dir_entry.attributes & 0x04) != 0;
                        let is_special = matches!(
                            dir_entry.name.to_lowercase().as_str(),
                            "desktop.ini"
                                | "thumbs.db"
                                | "$recycle.bin"
                                | "system volume information"
                        );
                        if !is_hidden
                            && !is_system
                            && !is_special
                            && !dir_entry.name.starts_with('.')
                        {
                            let full_path = PathBuf::from(&base_path).join(&dir_entry.name);
                            let mut is_dir = dir_entry.is_dir;
                            if !is_dir && dir_entry.name.to_lowercase().ends_with(".zip") {
                                is_dir = true;
                            }
                            let is_onedrive = onedrive::is_onedrive_path(&full_path);
                            let sync_status =
                                onedrive::get_sync_status(dir_entry.attributes, is_onedrive);
                            let entry = crate::domain::file_entry::FileEntry {
                                path: full_path,
                                name: dir_entry.name,
                                is_dir,
                                size: if is_dir { 0 } else { dir_entry.size },
                                modified: dir_entry.modified,
                                folder_cover: None,
                                drive_info: None,
                                sync_status,
                                deletion_date: None,
                                recycle_original_path: None,
                            };
                            all_entries_disk.push(entry.clone());
                            batch.push(entry);
                            if batch.len() >= batch_size {
                                let folders: Vec<PathBuf> = batch
                                    .iter()
                                    .filter(|e| e.is_dir)
                                    .map(|e| e.path.clone())
                                    .collect();
                                if !folders.is_empty() {
                                    let covers = disk_cache.get_folder_covers(&folders);
                                    for item in batch.iter_mut() {
                                        if item.is_dir {
                                            if let Some(cover) = covers.get(&item.path) {
                                                item.folder_cover = Some(cover.clone());
                                            }
                                        }
                                    }
                                }
                                let batch_len = batch.len();
                                let _ = file_entry_sender.send((my_gen, batch.clone()));
                                batch_tracker.record_batch(batch_start.elapsed(), batch_len);
                                batch_size = batch_tracker.batch_size();
                                batch_start = std::time::Instant::now();
                                batch.clear();
                                ctx.request_repaint();
                            }
                        }
                    }
                    if !batch.is_empty() && gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
                        let folders: Vec<PathBuf> = batch
                            .iter()
                            .filter(|e| e.is_dir)
                            .map(|e| e.path.clone())
                            .collect();
                        if !folders.is_empty() {
                            let covers = disk_cache.get_folder_covers(&folders);
                            for item in batch.iter_mut() {
                                if item.is_dir {
                                    if let Some(cover) = covers.get(&item.path) {
                                        item.folder_cover = Some(cover.clone());
                                    }
                                }
                            }
                        }
                        let batch_len = batch.len();
                        let _ = file_entry_sender.send((my_gen, batch));
                        batch_tracker.record_batch(batch_start.elapsed(), batch_len);
                        ctx.request_repaint();
                    }
                    if gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
                        let _ = file_entry_sender.send((my_gen, Vec::new()));
                        ctx.request_repaint();
                    }
                    if gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
                        directory_cache.put(PathBuf::from(&base_path), all_entries_disk.clone());
                        if let Some(di) = &directory_index_opt {
                            let indexed: Vec<IndexedFile> = all_entries_disk
                                .iter()
                                .map(|e| IndexedFile {
                                    name: e.name.clone(),
                                    size: e.size,
                                    modified: e.modified,
                                    is_dir: e.is_dir,
                                })
                                .collect();
                            let _ = di.put_directory(
                                &PathBuf::from(&base_path),
                                &indexed,
                                scan_start.elapsed().as_millis() as u64,
                            );
                        }
                    }
                    if !is_ssd && gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
                        let subdirs: Vec<PathBuf> = all_entries_disk
                            .iter()
                            .filter(|e| e.is_dir)
                            .take(5)
                            .map(|e| e.path.clone())
                            .collect();
                        if !subdirs.is_empty() {
                            let _ = prefetch_sender.send(PrefetchMessage::Prefetch(subdirs));
                        }
                    }
                    return;
                }
            }

            // Prepara busca Win32
            let search_path = if base_path.ends_with('\\') {
                format!("{}*", base_path)
            } else {
                format!("{}\\*", base_path)
            };
            let wide_path: Vec<u16> = search_path
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let mut find_data = WIN32_FIND_DATAW::default();

            // Check if we're in a OneDrive folder (for sync status)
            let is_onedrive = onedrive::is_onedrive_path(&PathBuf::from(&current_path));

            unsafe {
                // SAFETY: `wide_path` is a null-terminated UTF-16 string buffer.
                // `find_data` is a valid pointer to a `WIN32_FIND_DATAW` struct.
                // The handle returned is checked for validity and closed via `FindClose`
                // before the scope ends.
                if let Ok(handle) = FindFirstFileW(PCWSTR(wide_path.as_ptr()), &mut find_data) {
                    loop {
                        // Verifica se a geração mudou -> Aborta scan antigo
                        if gen_clone.load(AtomicOrdering::Relaxed) != my_gen {
                            break;
                        }

                        let len = find_data
                            .cFileName
                            .iter()
                            .position(|&c| c == 0)
                            .unwrap_or(find_data.cFileName.len());
                        let filename = std::ffi::OsString::from_wide(&find_data.cFileName[0..len])
                            .to_string_lossy()
                            .into_owned();

                        if filename != "." && filename != ".." {
                            let attrs = find_data.dwFileAttributes;
                            let full_path = PathBuf::from(&base_path).join(&filename);

                            // PERFORMANCE: Use basic attributes from FindFirstFileW/FindNextFileW.
                            // They already contain OneDrive flags (RECALL_ON_OPEN, RECALL_ON_DATA_ACCESS, PINNED).
                            // Calling GetFileAttributesW() again is redundant and adds 2ms per file!
                            //
                            // OLD CODE (removed - was causing 98% of scan time on OneDrive):
                            // let extended_attrs = if is_onedrive {
                            //     let path_wide: Vec<u16> = full_path.to_string_lossy()...
                            //     GetFileAttributesW(...)  // ← 2ms syscall PER FILE!
                            // } else {
                            //     attrs
                            // };
                            let extended_attrs = attrs;

                            // Filtros: hidden/system files
                            let is_hidden = (extended_attrs & FILE_ATTRIBUTE_HIDDEN.0) != 0;
                            let is_system = (extended_attrs & FILE_ATTRIBUTE_SYSTEM.0) != 0;
                            let is_special = matches!(
                                filename.to_lowercase().as_str(),
                                "desktop.ini"
                                    | "thumbs.db"
                                    | "$recycle.bin"
                                    | "system volume information" // Re-adicionado "System Volume Information" para garantir compatibilidade
                            );

                            if !is_hidden && !is_system && !is_special && !filename.starts_with('.')
                            {
                                let mut is_dir = (extended_attrs & FILE_ATTRIBUTE_DIRECTORY.0) != 0;

                                // Treat ZIP files as navigable folders
                                if !is_dir && filename.to_lowercase().ends_with(".zip") {
                                    is_dir = true;
                                }

                                let is_zip = filename.to_lowercase().ends_with(".zip");
                                let size = if is_dir && !is_zip {
                                    0
                                } else {
                                    ((find_data.nFileSizeHigh as u64) << 32)
                                        | (find_data.nFileSizeLow as u64)
                                };

                                let ft = find_data.ftLastWriteTime;
                                let windows_ticks =
                                    ((ft.dwHighDateTime as u64) << 32) | (ft.dwLowDateTime as u64);
                                let modified = if windows_ticks > 116444736000000000 {
                                    (windows_ticks - 116444736000000000) / 10_000_000
                                } else {
                                    0
                                };

                                // OPTIMIZATION: Folder cover loading moved to batch (outside loop)
                                // to avoid N+1 queries in SQLite.
                                let folder_cover = None;

                                // Check if file is currently open (being used)
                                let sync_status =
                                    onedrive::get_sync_status(extended_attrs, is_onedrive);

                                // DISABLED: is_file_open() is EXTREMELY slow on OneDrive (28ms per file!)
                                // It tries to open file handles which triggers sync/network checks.
                                // Windows Explorer doesn't do this - it only uses file attributes.
                                //
                                // OLD CODE (removed for performance):
                                // if is_onedrive && !is_dir && sync_status != SyncStatus::None {
                                //     if onedrive::is_file_open(&full_path) {
                                //         sync_status = SyncStatus::Syncing;
                                //     }
                                // }

                                let entry = FileEntry {
                                    path: full_path,
                                    name: filename,
                                    is_dir,
                                    size,
                                    modified,
                                    folder_cover,
                                    drive_info: None,
                                    sync_status,
                                    deletion_date: None,
                                    recycle_original_path: None,
                                };

                                // Adiciona ao lote
                                all_entries_disk.push(entry.clone());
                                batch.push(entry);

                                // SE o lote encheu, envia e limpa (tamanho adaptado para SSD/HDD)
                                if batch.len() >= batch_size {
                                    // PRE-FETCH COVERS (Batch Optimization)
                                    let folders: Vec<PathBuf> = batch
                                        .iter()
                                        .filter(|e| e.is_dir)
                                        .map(|e| e.path.clone())
                                        .collect();

                                    if !folders.is_empty() {
                                        let covers = disk_cache.get_folder_covers(&folders);
                                        for item in batch.iter_mut() {
                                            if item.is_dir {
                                                if let Some(cover) = covers.get(&item.path) {
                                                    item.folder_cover = Some(cover.clone());
                                                }
                                            }
                                        }
                                    }

                                    let batch_len = batch.len();
                                    let _ = file_entry_sender.send((my_gen, batch.clone()));
                                    batch_tracker.record_batch(batch_start.elapsed(), batch_len);
                                    batch_size = batch_tracker.batch_size();
                                    batch_start = std::time::Instant::now();
                                    batch.clear();
                                    ctx.request_repaint(); // Acorda a UI para mostrar progresso
                                }
                            }
                        }

                        if FindNextFileW(handle, &mut find_data).is_err() {
                            break;
                        }
                    }
                    let _ = FindClose(handle);
                }
            }

            // Envia o restante (último lote) se sobrou algo e a geração ainda é válida
            if !batch.is_empty() && gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
                // PRE-FETCH COVERS (Batch Optimization) - Last batch
                let folders: Vec<PathBuf> = batch
                    .iter()
                    .filter(|e| e.is_dir)
                    .map(|e| e.path.clone())
                    .collect();

                if !folders.is_empty() {
                    let covers = disk_cache.get_folder_covers(&folders);
                    for item in batch.iter_mut() {
                        if item.is_dir {
                            if let Some(cover) = covers.get(&item.path) {
                                item.folder_cover = Some(cover.clone());
                            }
                        }
                    }
                }

                let batch_len = batch.len();
                let _ = file_entry_sender.send((my_gen, batch));
                batch_tracker.record_batch(batch_start.elapsed(), batch_len);
                ctx.request_repaint();
            }

            // Envia vetor VAZIO para sinalizar FIM do carregamento (apenas se a geração for a mesma)
            if gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
                let scan_elapsed = scan_start.elapsed();
                eprintln!(
                    "[PERF] Folder scan complete: {:?} took {:.2}s",
                    current_path,
                    scan_elapsed.as_secs_f64()
                );
                let _ = file_entry_sender.send((my_gen, Vec::new()));
                ctx.request_repaint();
            }

            if gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
                directory_cache.put(PathBuf::from(&base_path), all_entries_disk.clone());
                if let Some(di) = &directory_index_opt {
                    let indexed: Vec<IndexedFile> = all_entries_disk
                        .iter()
                        .map(|e| IndexedFile {
                            name: e.name.clone(),
                            size: e.size,
                            modified: e.modified,
                            is_dir: e.is_dir,
                        })
                        .collect();
                    let _ = di.put_directory(
                        &PathBuf::from(&base_path),
                        &indexed,
                        scan_start.elapsed().as_millis() as u64,
                    );
                }
            }
            if !is_ssd && gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
                let subdirs: Vec<PathBuf> = all_entries_disk
                    .iter()
                    .filter(|e| e.is_dir)
                    .take(5)
                    .map(|e| e.path.clone())
                    .collect();
                if !subdirs.is_empty() {
                    let _ = prefetch_sender.send(PrefetchMessage::Prefetch(subdirs));
                }
            }
        });
    }

    pub fn trigger_manual_refresh(&mut self) {
        if self.is_computer_view {
            let _ = self.reload_drive_list();
            self.setup_computer_view();
            self.last_drive_refresh = Instant::now();
        } else if self.is_recycle_bin_view {
            self.setup_recycle_bin_view();
        } else {
            self.load_folder(true);
        }
    }
}
