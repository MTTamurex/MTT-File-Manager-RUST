//! Folder loading: load_folder, filter_items, sort_items, refresh
//!
//! This module handles scanning folders, filtering results, sorting, and manual refresh triggers.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering as AtomicOrdering;
use std::time::Instant;

use windows::Win32::Storage::FileSystem::*;
use windows::core::PCWSTR;
use std::os::windows::ffi::OsStringExt;

use crate::app::state::ImageViewerApp;
use crate::application::sorting;
use crate::domain::file_entry::FileEntry;
use crate::infrastructure::onedrive;

impl ImageViewerApp {
    pub fn filter_items(&mut self) {
        self.items = Arc::new(sorting::filter_items(&self.all_items, &self.search_query));
        self.total_items = self.items.len();
        self.sort_items();
    }

    /// Ordena itens baseado no modo atual e preferência de posição de pastas
    /// OTIMIZADO: Usa par_sort_by para listas >5000 itens (rayon)
    pub fn sort_items(&mut self) {
        let mut items = (*self.items).clone();
        sorting::sort_items(
            &mut items,
            self.sort_mode,
            self.sort_descending,
            self.folders_position,
        );
        self.items = Arc::new(items);
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

        // 1. Limpeza de Estado (UI Thread)
        if force_refresh {
            self.cache_manager.texture_cache.clear();
            self.cache_manager.folder_preview_cache.clear();
        }

        self.items = Arc::new(Vec::new()); // Novo Arc vazio (antigo é dropped automaticamente)
        self.all_items.clear(); // Limpa backup mestre também
        self.cache_manager.loading_set.clear(); // Limpa apenas requisições pendentes, mantém cache de texturas
        self.scanned_folders.clear();
        self.selected_item = None;
        self.is_loading_folder = true;
        self.total_items = 0;

        let my_gen = self.generation;
        let gen_clone = self.current_generation.clone();
        let current_path = self.current_path.clone();
        let file_entry_sender = self.file_entry_sender.clone();
        let ctx = self.ui_ctx.clone();
        let disk_cache = self.disk_cache.clone();

        // STREAMING BATCH LOADING: Envia lotes de 250 itens progressivamente
        std::thread::spawn(move || {
            let scan_start = std::time::Instant::now();
            eprintln!("[PERF] Starting folder scan: {:?}", current_path);
            // Buffer para envio em lotes
            let mut batch = Vec::with_capacity(250);

            // Normaliza o path base: drive roots precisam de trailing backslash
            // Ex: "Z:" -> "Z:\\" para que PathBuf::join funcione corretamente
            let base_path = if current_path.len() == 2 && current_path.ends_with(':') {
                format!("{}\\", current_path)
            } else {
                current_path.clone()
            };

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
                                let is_dir = (extended_attrs & FILE_ATTRIBUTE_DIRECTORY.0) != 0;

                                let size = if is_dir {
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

                                let folder_cover = if is_dir {
                                    disk_cache.get_folder_cover(&full_path)
                                } else {
                                    None
                                };

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
                                };

                                // Adiciona ao lote
                                batch.push(entry);

                                // SE o lote encheu (250 itens), envia e limpa
                                if batch.len() >= 250 {
                                    let _ = file_entry_sender.send((my_gen, batch.clone()));
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
                let _ = file_entry_sender.send((my_gen, batch));
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
        });
    }

    pub fn trigger_manual_refresh(&mut self) {
        if self.is_computer_view {
            let _ = self.reload_drive_list();
            self.setup_computer_view();
            self.last_drive_refresh = Instant::now();
        } else {
            self.load_folder(true);
        }
    }
}
