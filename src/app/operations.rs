// Application Operations - All ImageViewerApp methods
// This file contains the business logic and operations for the file manager

//! Application operations and business logic.
//!
//! This module implements the `ImageViewerApp` methods for handling file operations,
//! navigation, searching, sorting, and UI interactions. It acts as the controller layer.

// use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering as AtomicOrdering;
use std::sync::Arc;
use std::time::Instant;

use crate::app::state::ImageViewerApp;
use crate::application::file_operations;
use crate::application::sorting;
use crate::domain::file_entry::{FileEntry, FoldersPosition, IconSize, SortMode, ViewMode};
use crate::infrastructure::onedrive;
use crate::infrastructure::windows as windows_infra;

use std::os::windows::ffi::OsStringExt;
use windows::core::PCWSTR;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
};
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::FindWindowW;

use notify::{RecursiveMode, Watcher};
use std::time::{Duration, UNIX_EPOCH};

use crate::infrastructure::windows::{extract_file_icon_by_path, open_with_shell};
use crate::ui::theme; // For constants like DRIVE_REFRESH_INTERVAL_MS
use eframe::egui; // For UI-related methods

const DRIVE_REFRESH_INTERVAL_MS: u64 = crate::ui::theme::DRIVE_REFRESH_MS;

// Re-export helper from main.rs if needed
// Function removed: using crate::infrastructure::windows::get_all_drives instead

// PlaceHolder - este arquivo serÃ¡ reconstruÃ­do a partir do operations_temp.rs
// com os imports corretos
impl ImageViewerApp {
    pub fn delete_with_shell_for_idx(&mut self, idx: Option<usize>) {
        let target_idx = idx.or(self.selected_item);
        if let Some(idx) = target_idx {
            if let Some(item) = self.items.get(idx) {
                // Delegate to application layer
                if let Ok(true) = file_operations::delete_with_shell(&item.path, self.native_hwnd) {
                    // Limpa cache do item deletado
                    self.disk_cache.remove_cache_for_path(&item.path);

                    // O watcher vai cuidar do refresh, mas podemos limpar a seleÃ§Ã£o
                    if self.selected_item == Some(idx) {
                        self.selected_item = None;
                        self.selected_file = None;
                    }
                }
            }
        }
    }

    pub fn restore_from_recycle_bin(&mut self, physical_path: &Path) {
        use crate::infrastructure::windows::recycle_bin::{
            enumerate_recycle_bin, restore_from_recycle_bin,
        };

        // Get the original path from RecycleBinItem by re-enumerating
        // This ensures we get the correct original_path stored in the $I file
        let original_path = if let Ok(recycle_items) = enumerate_recycle_bin() {
            recycle_items
                .iter()
                .find(|item| item.physical_path == physical_path)
                .map(|item| item.original_path.clone())
        } else {
            None
        };

        if let Some(item) = self.items.iter().find(|i| i.path == physical_path) {
            let original_path = original_path.unwrap_or_else(|| {
                // Fallback: use Desktop if we can't find original path
                PathBuf::from("C:\\Users\\Public\\Desktop").join(item.name.clone())
            });

            match restore_from_recycle_bin(physical_path, &original_path) {
                Ok(_) => {
                    self.notifications
                        .push(crate::application::AppNotification::success(format!(
                            "'{}' restaurado com sucesso",
                            item.name
                        )));
                    // Refresh recycle bin view
                    self.setup_recycle_bin_view();
                }
                Err(e) => {
                    self.notifications
                        .push(crate::application::AppNotification::error(format!(
                            "Erro ao restaurar: {}",
                            e
                        )));
                }
            }
        }
    }

    pub fn delete_permanently(&mut self, physical_path: &Path) {
        use crate::infrastructure::windows::recycle_bin::delete_permanently;

        if let Some(item) = self.items.iter().find(|i| i.path == physical_path) {
            let item_name = item.name.clone();

            match delete_permanently(physical_path) {
                Ok(_) => {
                    self.notifications
                        .push(crate::application::AppNotification::success(format!(
                            "'{}' excluído permanentemente",
                            item_name
                        )));
                    // Refresh recycle bin view
                    self.setup_recycle_bin_view();
                }
                Err(e) => {
                    self.notifications
                        .push(crate::application::AppNotification::error(format!(
                            "Erro ao excluir: {}",
                            e
                        )));
                }
            }
        }
    }

    pub fn empty_recycle_bin(&mut self) {
        use crate::infrastructure::windows::recycle_bin::empty_recycle_bin;

        match empty_recycle_bin() {
            Ok(_) => {
                self.notifications
                    .push(crate::application::AppNotification::success(
                        "Lixeira esvaziada com sucesso".to_string(),
                    ));
                // Refresh recycle bin view
                self.setup_recycle_bin_view();
            }
            Err(e) => {
                self.notifications
                    .push(crate::application::AppNotification::error(format!(
                        "Erro ao esvaziar lixeira: {}",
                        e
                    )));
            }
        }
    }

    pub fn show_properties_for_idx(&mut self, idx: Option<usize>) {
        let target_idx = idx.or(self.selected_item);
        if let Some(idx) = target_idx {
            if let Some(item) = self.items.get(idx) {
                let path = item.path.clone();
                // We'll use the shell properties dialog
                let _ = crate::infrastructure::windows::native_menu::show_properties_dialog(
                    self.native_hwnd.unwrap_or_default(),
                    &path,
                );
            }
        }
    }

    pub fn create_new_folder(&mut self) {
        let base_path = PathBuf::from(&self.current_path);

        match file_operations::create_new_folder(&base_path) {
            Ok(full_path) => {
                let new_folder_name = full_path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();

                // CRITICAL: Immediately create entry to allow renaming
                let new_item = FileEntry::from_path(full_path.clone(), true);

                self.all_items.push(new_item);
                self.filter_items();
                self.sort_items();

                // Find index in filtered vector
                if let Some(idx) = self.items.iter().position(|i| i.path == full_path) {
                    self.selected_item = Some(idx);
                    self.selected_file = Some(self.items[idx].clone());
                    self.renaming_state = Some((idx, new_folder_name));
                    self.focus_rename = true;
                }

                // Request background real load to sync with disk
                self.load_folder(false);
            }
            Err(e) => {
                eprintln!("Erro ao criar pasta: {}", e);
            }
        }
    }

    // ===== CLIPBOARD OPERATIONS (Ctrl+C, Ctrl+X, Ctrl+V) =====

    /// Copiar: Coloca o arquivo no clipboard do Windows (CF_HDROP format)
    /// Copiar: Coloca o arquivo no clipboard do Windows e interno
    pub fn command_copy(&mut self, idx: Option<usize>) {
        if let Some(idx) = idx.or(self.selected_item) {
            if let Some(item) = self.items.get(idx) {
                self.clipboard.copy(&item.path);
            }
        }
    }

    /// Recortar: Coloca o arquivo no clipboard do Windows com flag de MOVE
    pub fn command_cut(&mut self, idx: Option<usize>) {
        let target_idx = idx.or(self.selected_item);
        if let Some(idx) = target_idx {
            if let Some(item) = self.items.get(idx) {
                self.clipboard.cut(&item.path);
            }
        }
    }

    /// Colar: LÃª do clipboard usando ClipboardManager
    pub fn command_paste(&mut self, idx: Option<usize>) {
        eprintln!("[DEBUG] command_paste called with idx: {:?}", idx);

        // Destination folder
        let dest_folder = if let Some(idx) = idx {
            if let Some(item) = self.items.get(idx) {
                if item.is_dir {
                    item.path.clone()
                } else {
                    PathBuf::from(&self.current_path)
                }
            } else {
                PathBuf::from(&self.current_path)
            }
        } else {
            PathBuf::from(&self.current_path)
        };

        match self.clipboard.paste(&dest_folder, self.native_hwnd) {
            Ok(true) => {
                // Move successful
                self.load_folder(false);
                self.context_menu.target_path = None;
            }
            Ok(false) => {
                // Copy successful
                self.load_folder(false);
                self.context_menu.target_path = None;
            }
            Err(e) => {
                eprintln!("[CLIPBOARD ERROR] {}", e);
            }
        }
    }

    /// Filtra itens baseado na query de busca e reaplica ordenaÃ§Ã£o
    /// Filtra itens baseado na query de busca e reaplica ordenaÃ§Ã£o
    pub fn filter_items(&mut self) {
        self.items = Arc::new(sorting::filter_items(&self.all_items, &self.search_query));
        self.total_items = self.items.len();
        self.sort_items();
    }

    /// Ordena itens baseado no modo atual e preferÃªncia de posiÃ§Ã£o de pastas
    /// OTIMIZADO: Usa par_sort_by para listas >5000 itens (rayon)
    pub fn sort_items(&mut self) {
        // Delegate to pure function in sorting module
        sorting::sort_items(
            Arc::make_mut(&mut self.items).as_mut_slice(),
            self.sort_mode,
            self.sort_descending,
            self.folders_position,
        );
    }

    /// Salva as preferÃªncias atuais no SQLite
    pub fn save_preferences(&self) {
        let sort_mode_str = match self.sort_mode {
            SortMode::Name => "name",
            SortMode::Date => "date",
            SortMode::Size => "size",
            SortMode::Type => "type",
        };
        self.disk_cache.set_preference("sort_mode", sort_mode_str);

        self.disk_cache.set_preference(
            "sort_descending",
            if self.sort_descending {
                "true"
            } else {
                "false"
            },
        );

        let folders_pos_str = match self.folders_position {
            FoldersPosition::First => "first",
            FoldersPosition::Last => "last",
            FoldersPosition::Mixed => "mixed",
        };
        self.disk_cache
            .set_preference("folders_position", folders_pos_str);

        // UI preferences
        self.disk_cache
            .set_preference("thumbnail_size", &self.thumbnail_size.to_string());

        let view_mode_str = match self.view_mode {
            ViewMode::Grid => "grid",
            ViewMode::List => "list",
        };
        self.disk_cache.set_preference("view_mode", view_mode_str);

        self.disk_cache.set_preference(
            "show_preview_panel",
            if self.show_preview_panel {
                "true"
            } else {
                "false"
            },
        );

        // Window state persistence
        self.disk_cache
            .set_preference("window_width", &self.saved_window_width.to_string());
        self.disk_cache
            .set_preference("window_height", &self.saved_window_height.to_string());
        self.disk_cache.set_preference(
            "window_is_maximized",
            if self.saved_is_maximized {
                "true"
            } else {
                "false"
            },
        );

        // Sidebar widths persistence - sÃ³ salva valores vÃ¡lidos
        let left_to_save = self.sidebar_left_width.max(150.0);
        let right_to_save = self.sidebar_right_width.max(250.0);
        self.disk_cache
            .set_preference("sidebar_left_width", &left_to_save.to_string());
        self.disk_cache
            .set_preference("sidebar_right_width", &right_to_save.to_string());
    }

    /// Requisita scan assÃƒÂ­ncrono de uma pasta para descobrir primeira imagem.
    /// OTIMIZADO: Envia mensagem para worker ÃƒÂºnico (zero overhead de threads)
    pub fn request_folder_scan(&self, folder_path: PathBuf) {
        // Apenas envia para fila - worker processa em background
        let _ = self.cover_worker_sender.send(folder_path);
    }

    pub fn load_folder(&mut self, force_refresh: bool) {
        self.generation += 1; // Incrementa a geraÃ§Ã£o local
        self.current_generation
            .store(self.generation, AtomicOrdering::Relaxed); // Sincroniza com workers

        // 1. Limpeza de Estado (UI Thread)
        if force_refresh {
            self.cache_manager.texture_cache.clear();
            self.cache_manager.folder_preview_cache.clear();
        }

        self.items = Arc::new(Vec::new()); // Novo Arc vazio (antigo Ã© dropped automaticamente)
        self.all_items.clear(); // Limpa backup mestre tambÃ©m
        self.cache_manager.loading_set.clear(); // Limpa apenas requisiÃ§Ãµes pendentes, mantÃ©m cache de texturas
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
                        // Verifica se a geraÃ§Ã£o mudou -> Aborta scan antigo
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
                            //     GetFileAttributesW(...)  // â† 2ms syscall PER FILE!
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

            // Envia o restante (Ãºltimo lote) se sobrou algo e a geraÃ§Ã£o ainda Ã© vÃ¡lida
            if !batch.is_empty() && gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
                let _ = file_entry_sender.send((my_gen, batch));
                ctx.request_repaint();
            }

            // Envia vetor VAZIO para sinalizar FIM do carregamento (apenas se a geraÃ§Ã£o for a mesma)
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

    /// Navega para um caminho, adicionando ao histÃƒÂ³rico (corta histÃƒÂ³rico futuro)
    pub fn navigate_to(&mut self, path: &str) {
        // Normaliza paths de drive roots: garante que "Z:" sempre vire "Z:\"
        // Isso corrige o bug do PathBuf::join nÃ£o adicionar backslash
        let normalized_path = if path.len() >= 2 && path.chars().nth(1) == Some(':') {
            // Ã‰ um path Windows com letra de drive
            if path.len() == 2 {
                // Apenas "Z:" -> "Z:\"
                format!("{}\\", path)
            } else if path.chars().nth(2) != Some('\\') {
                // "Z:folder" -> "Z:\folder" (corrige path malformado)
                format!("{}\\{}", &path[0..2], &path[2..])
            } else {
                path.to_string()
            }
        } else {
            path.to_string()
        };

        // Se jÃƒÂ¡ estamos nesse caminho, nÃƒÂ£o faz nada
        if self.current_path == normalized_path {
            return;
        }

        // Adiciona novo caminho ao histÃƒÂ³rico
        self.navigation.navigate_to(normalized_path.clone());

        self.current_path = normalized_path.clone();
        self.path_input = normalized_path.clone();
        self.is_computer_view = false;
        self.is_recycle_bin_view = false; // Reset quando navega para qualquer pasta

        // SYNC TAB STATE
        self.sync_to_tab();

        self.reset_selection_and_search();

        // ATUALIZA O VIGIA
        self.watch_current_folder();

        self.load_folder(false);
    }

    pub fn go_back(&mut self) {
        if let Some(path) = self.navigation.go_back().cloned() {
            // Guarda o path atual antes de voltar (para invalidar o preview)
            let previous_path = std::path::PathBuf::from(&self.current_path);

            if path == "Este Computador" {
                // Invalida preview da pasta que estÃ¡vamos
                self.cache_manager.invalidate_folder_preview(&previous_path);

                // SYNC TAB STATE
                self.sync_to_tab();

                self.reset_selection_and_search();
                self.setup_computer_view();
            } else if path == "Lixeira" {
                // Invalida preview da pasta que estÃ¡vamos
                self.cache_manager.invalidate_folder_preview(&previous_path);

                self.reset_selection_and_search();
                self.setup_recycle_bin_view();
            } else {
                let new_path = std::path::PathBuf::from(&path);

                // Se estÃ¡vamos em uma subpasta do destino, invalida o preview dessa subpasta
                if previous_path.starts_with(&new_path) && previous_path != new_path {
                    self.cache_manager.invalidate_folder_preview(&previous_path);
                }

                self.current_path = path.clone();
                self.sync_to_tab();
                self.path_input = self.current_path.clone();
                self.is_computer_view = false;
                self.is_recycle_bin_view = false;
                self.reset_selection_and_search();
                self.watch_current_folder(); // Atualiza o watcher
                self.load_folder(false);
            }
        }
    }

    /// AvanÃ§a no histÃ³rico
    pub fn go_forward(&mut self) {
        if let Some(path) = self.navigation.go_forward().cloned() {
            // Guarda o path atual antes de avanÃ§ar (para invalidar o preview)
            let previous_path = std::path::PathBuf::from(&self.current_path);

            if path == "Este Computador" {
                self.cache_manager.invalidate_folder_preview(&previous_path);

                // SYNC TAB STATE
                self.sync_to_tab();

                self.reset_selection_and_search();
                self.setup_computer_view();
            } else if path == "Lixeira" {
                self.cache_manager.invalidate_folder_preview(&previous_path);

                self.reset_selection_and_search();
                self.setup_recycle_bin_view();
            } else {
                let new_path = std::path::PathBuf::from(&path);

                // Se o destino Ã© pai do path atual, invalida o preview do path atual
                if previous_path.starts_with(&new_path) && previous_path != new_path {
                    self.cache_manager.invalidate_folder_preview(&previous_path);
                }

                self.current_path = path.clone();
                self.sync_to_tab();
                self.path_input = self.current_path.clone();
                self.is_computer_view = false;
                self.is_recycle_bin_view = false;
                self.reset_selection_and_search();
                self.watch_current_folder(); // Atualiza o watcher
                self.load_folder(false);
            }
        }
    }

    /// Navega para "Este Computador" view (adicionando ao histÃ³rico)
    pub fn navigate_to_computer(&mut self) {
        if self.is_computer_view {
            return;
        }

        self.reset_selection_and_search();

        // Adiciona ao histÃ³rico (via manager)
        self.navigation.navigate_to("Este Computador".to_string());

        // SYNC TAB STATE
        self.tab_manager.active_mut().navigate_to("Este Computador");

        let _ = self.reload_drive_list();
        self.last_drive_refresh = Instant::now();
        self.setup_computer_view();
    }

    /// Navega para a Lixeira (adicionando ao histÃ³rico)
    pub fn navigate_to_recycle_bin(&mut self) {
        if self.is_recycle_bin_view {
            return;
        }

        self.reset_selection_and_search();

        // Adiciona ao histÃ³rico
        self.navigation.navigate_to("Lixeira".to_string());

        // SYNC TAB STATE
        self.tab_manager.active_mut().navigate_to("Lixeira");

        self.setup_recycle_bin_view();
    }

    /// Configura a visÃ£o da Lixeira de forma ASSÃNCRONA
    pub fn setup_recycle_bin_view(&mut self) {
        self.current_path = "Lixeira".to_string();
        self.is_computer_view = false;
        self.is_recycle_bin_view = true;
        self.path_input = "Lixeira".to_string();
        self.is_loading_folder = true;
        self.items = Arc::new(Vec::new());
        self.all_items.clear();
        self.total_items = 0;

        // Incrementa geraÃ§Ã£o para invalidar thumbnails antigos
        self.generation += 1;
        self.current_generation
            .store(self.generation, AtomicOrdering::Relaxed);

        let my_gen = self.generation;
        let gen_clone = self.current_generation.clone();
        let file_entry_sender = self.file_entry_sender.clone();
        let ctx = self.ui_ctx.clone();

        // Carrega itens da lixeira em thread separada (ASYNC) com batching
        std::thread::spawn(move || {
            use crate::infrastructure::windows::recycle_bin::enumerate_recycle_bin;

            // Enumera itens da lixeira via COM
            match enumerate_recycle_bin() {
                Ok(recycle_items) => {
                    const BATCH_SIZE: usize = 100;
                    let mut batch = Vec::with_capacity(BATCH_SIZE);

                    for item in recycle_items {
                        // Verifica se a geraÃ§Ã£o ainda Ã© vÃ¡lida (cancelamento rÃ¡pido)
                        if gen_clone.load(AtomicOrdering::Relaxed) != my_gen {
                            return;
                        }

                        // Cria um path "virtual" baseado na extensÃ£o para carregar Ã­cone correto
                        // O path real nÃ£o existe mais, mas o Ã­cone Ã© baseado na extensÃ£o
                        // O path real ($R) Ã© necessÃ¡rio para ler a data de exclusÃ£o ($I creation time)
                        // Se physical_path estiver vazio (falha ao ler), usamos a lÃ³gica antiga de dummy.
                        let file_path = if !item.physical_path.as_os_str().is_empty() {
                            item.physical_path.clone()
                        } else if item.is_directory {
                            PathBuf::from("C:\\folder")
                        } else if !item.extension.is_empty() {
                            PathBuf::from(format!("dummy{}", item.extension))
                        } else {
                            item.original_path.clone()
                        };

                        let entry = FileEntry {
                            path: file_path, // Path fÃ­sico ($R) para permitir get_deletion_date
                            name: item.name,
                            is_dir: item.is_directory,
                            size: item.size,
                            modified: 0,
                            folder_cover: None,
                            drive_info: None,
                            sync_status: crate::domain::file_entry::SyncStatus::None,
                            deletion_date: Some(item.date_deleted),
                        };
                        batch.push(entry);

                        // Envia batch quando cheio
                        if batch.len() >= BATCH_SIZE {
                            if gen_clone.load(AtomicOrdering::Relaxed) != my_gen {
                                return;
                            }
                            let _ = file_entry_sender.send((my_gen, std::mem::take(&mut batch)));
                            ctx.request_repaint();
                            batch = Vec::with_capacity(BATCH_SIZE);
                        }
                    }

                    // Envia itens restantes
                    if !batch.is_empty() && gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
                        let _ = file_entry_sender.send((my_gen, batch));
                        ctx.request_repaint();
                    }

                    // Sinal de fim do carregamento
                    if gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
                        let _ = file_entry_sender.send((my_gen, Vec::new()));
                        ctx.request_repaint();
                    }
                }
                Err(e) => {
                    eprintln!("[RECYCLE BIN] Erro ao enumerar: {:?}", e);
                    let _ = file_entry_sender.send((my_gen, Vec::new()));
                    ctx.request_repaint();
                }
            }
        });
    }

    /// Configura a visÃ£o de "Este Computador" sem afetar o histÃ³rico
    pub fn setup_computer_view(&mut self) {
        // Set computer view
        self.current_path = "Este Computador".to_string();
        self.is_computer_view = true;
        self.is_recycle_bin_view = false;
        self.path_input = "Este Computador".to_string();

        // ALWAYS reload drives to ensure fresh data
        let _ = self.reload_drive_list();

        // Populate items with drives
        use crate::domain::file_entry::DriveInfo;
        use crate::infrastructure::windows::get_volume_info;

        let mut computer_items = Vec::new();
        for (path, label) in &self.disks {
            let vol = get_volume_info(path);
            let drive_type = windows_infra::detect_drive_type(path);
            let mut entry = FileEntry::from_path(PathBuf::from(path), true);
            entry.name = label.clone();
            entry.drive_info = Some(DriveInfo {
                file_system: vol.file_system,
                total_space: vol.total_space,
                free_space: vol.free_space,
                drive_type,
            });
            computer_items.push(entry);
        }

        self.all_items = computer_items.clone();
        self.items = Arc::new(computer_items);
        self.reset_selection_and_search();
        self.total_items = self.disks.len();
        self.is_loading_folder = false; // CRITICAL: Clear loading state for computer view
    }

    pub fn reload_drive_list(&mut self) -> bool {
        let new_disks = crate::infrastructure::windows::get_all_drives();
        if new_disks != self.disks {
            self.disks = new_disks;
            return true;
        }
        false
    }

    pub fn refresh_drives_if_needed(&mut self) {
        if self.last_drive_refresh.elapsed() >= Duration::from_millis(DRIVE_REFRESH_INTERVAL_MS) {
            self.last_drive_refresh = Instant::now();
            if self.reload_drive_list() && self.is_computer_view {
                self.setup_computer_view();
            }
        }
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

    /// Sincroniza o estado atual do app para a aba ativa
    pub fn sync_to_tab(&mut self) {
        let active = self.tab_manager.active_mut();
        active.path = self.current_path.clone();
        active.path_input = self.path_input.clone();
        active.is_computer_view = self.is_computer_view;
        active.navigation = self.navigation.clone();
        active.items = self.items.clone();
        active.all_items = self.all_items.clone();
        active.selected_item = self.selected_item;
        active.selected_file = self.selected_file.clone();
        active.search_query = self.search_query.clone();
        active.scroll_to_selected = self.scroll_to_selected;

        // No Windows, Path::new("Este Computador").file_name() ÃƒÂ© None
        if active.is_computer_view {
            active.title = "Este Computador".to_string();
        } else {
            active.title = Path::new(&active.path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| active.path.clone());
        }
    }

    /// Sincroniza o estado da aba ativa para o app
    pub fn sync_from_tab(&mut self) {
        // Clonamos o estado da aba para evitar problemas de borrow checker ao atualizar self
        let active = self.tab_manager.active().clone();
        self.current_path = active.path;
        self.path_input = active.path_input;
        self.is_computer_view = active.is_computer_view;
        self.navigation = active.navigation.clone();
        self.items = active.items;
        self.all_items = active.all_items;
        self.selected_item = active.selected_item;
        self.selected_file = active.selected_file;
        self.search_query = active.search_query;
        self.scroll_to_selected = active.scroll_to_selected;

        self.watch_current_folder();
    }

    /// Sobe um nÃ­vel (adiciona ao histÃ³rico)
    pub fn go_up_one_level(&mut self) {
        if let Some(parent) = Path::new(&self.current_path).parent() {
            let parent_str = parent.to_string_lossy().to_string();
            // No Windows, parent de "C:\" Ã© vazio ou "." dependendo de como foi criado
            if !parent_str.is_empty() && parent_str != "." && parent_str != self.current_path {
                self.navigate_to(&parent_str);
                return;
            }
        }

        // Se jÃ¡ estamos no root de um drive ou local invÃ¡lido, vai para Computador
        if !self.is_computer_view {
            self.navigate_to_computer();
        }
    }

    /// Configura o monitoramento da pasta atual
    pub fn watch_current_folder(&mut self) {
        let current_path = self.current_path.clone();

        // Canonicaliza o path para compatibilidade com Windows
        let path_to_watch = if let Ok(p) = Path::new(&current_path).canonicalize() {
            p
        } else {
            PathBuf::from(&current_path)
        };

        // Se o watcher jÃ¡ existe, apenas troca o path monitorado
        if let Some(ref mut _watcher) = self.watcher {
            // Para de monitorar todos os paths antigos (o watcher pode ter mÃºltiplos)
            // Como nÃ£o temos referÃªncia ao path antigo, vamos recriar o watcher
            // (notify nÃ£o tem API para listar paths monitorados)
        }

        // Cria ou recria o watcher
        let tx = self.fs_event_sender.clone();
        let ctx_clone = self.ui_ctx.clone();

        let watcher_result =
            notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                let _ = tx.send(res);
                ctx_clone.request_repaint();
            });

        if let Ok(mut watcher) = watcher_result {
            if let Err(_e) = watcher.watch(&path_to_watch, RecursiveMode::NonRecursive) {
                // Silently fail - watcher is optional
            } else {
                self.watcher = Some(watcher);
            }
        }
    }

    /// Renomeia arquivo usando Shell API (suporta Undo/Ctrl+Z)
    pub fn rename_with_shell(&mut self, idx: usize) {
        if let Some((_, new_name)) = self.renaming_state.take() {
            if let Some(item) = self.items.get(idx) {
                let old_path = item.path.to_string_lossy().to_string();
                if let Some(parent) = item.path.parent() {
                    let new_path = parent.join(&new_name).to_string_lossy().to_string();

                    // Regra da API: Strings devem terminar com DOIS nulls (\0\0)
                    let mut from_vec: Vec<u16> = old_path.encode_utf16().collect();
                    from_vec.push(0);
                    from_vec.push(0);

                    let mut to_vec: Vec<u16> = new_path.encode_utf16().collect();
                    to_vec.push(0);
                    to_vec.push(0);

                    let mut op = SHFILEOPSTRUCTW {
                        hwnd: HWND(std::ptr::null_mut()),
                        wFunc: FO_RENAME,
                        pFrom: PCWSTR(from_vec.as_ptr()),
                        pTo: PCWSTR(to_vec.as_ptr()),
                        fFlags: FOF_ALLOWUNDO.0 as u16,
                        ..Default::default()
                    };

                    unsafe {
                        // SAFETY: `from_vec` and `to_vec` are properly double-null terminated wide strings
                        // as required by `SHFileOperationW`.
                        let result = SHFileOperationW(&mut op);
                        if result == 0 {
                            // Sucesso: Recarrega a pasta para atualizar a UI
                            self.load_folder(false);
                        } else {
                            eprintln!("Erro ao renomear via Shell: {}", result);
                        }
                    }
                }
            }
        }
    }

    pub fn can_go_back(&self) -> bool {
        self.navigation.can_go_back()
    }

    /// Pode avanÃƒÂ§ar no histÃƒÂ³rico?
    pub fn can_go_forward(&self) -> bool {
        self.navigation.can_go_forward()
    }

    pub fn request_thumbnail_load(&self, path: PathBuf) {
        // Envia pedido para o Worker Pool com a geraÃƒÂ§ÃƒÂ£o atual
        let _ = self.thumbnail_req_sender.send((path, self.generation));
    }

    pub fn request_folder_preview_load(&mut self, path: PathBuf) {
        if self
            .cache_manager
            .start_folder_preview_loading(path.clone())
        {
            let _ = self.folder_preview_sender.send(path);
        }
    }

    /// Captura e armazena o HWND nativo a partir do tÃ­tulo da janela principal.
    pub fn ensure_window_handle(&mut self, _frame: &eframe::Frame) {
        if self.native_hwnd.is_some() {
            return;
        }

        let title: Vec<u16> = "MTT File Manager"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let hwnd_result = unsafe { FindWindowW(None, PCWSTR(title.as_ptr())) };
        if let Ok(hwnd) = hwnd_result {
            if !hwnd.0.is_null() {
                self.native_hwnd = Some(hwnd);

                // Apply rounded corners (Windows 11 style)
                unsafe {
                    let corner_pref = DWMWCP_ROUND;
                    let result = DwmSetWindowAttribute(
                        hwnd,
                        DWMWA_WINDOW_CORNER_PREFERENCE,
                        &corner_pref as *const _ as *const _,
                        std::mem::size_of::<u32>() as u32,
                    );
                    if result.is_ok() {
                        eprintln!("[DWM] Rounded corners applied successfully");
                    } else {
                        eprintln!("[DWM] Failed to apply rounded corners: {:?}", result);
                    }
                }

                // Pre-initialize shell extensions so they're ready on first context menu
                crate::infrastructure::windows::native_menu::warmup_shell_extensions(hwnd);
            }
        }
    }

    /// Retorna icone para um arquivo, carregando sob demanda.
    /// Executaveis (.exe, .lnk, .ico) sao cacheados por path completo.
    /// Demais extensoes sao cacheadas por tipo.
    fn get_or_load_icon(
        &mut self,
        ctx: &egui::Context,
        path: &Path,
    ) -> Option<egui::TextureHandle> {
        let extension = path.extension()?.to_str()?.to_lowercase();

        // Decide cache key: path completo para executaveis, extensao para demais
        let cache_key = if matches!(extension.as_str(), "exe" | "lnk" | "ico") {
            // Cache por path completo - cada executavel tem icone unico
            path.to_string_lossy().to_string()
        } else {
            // Cache por extensao - todos .txt compartilham icone
            format!(".{}", extension)
        };

        // Cache hit? Clone do handle (barato)
        if let Some(texture) = self.cache_manager.icon_cache.get(&cache_key) {
            return Some(texture.clone());
        }

        // Cache miss -> carrega icone
        let thumbnail_size = self.thumbnail_size;
        let icon_size = if thumbnail_size < 100.0 {
            IconSize::Small
        } else {
            IconSize::Large
        };

        // Para executaveis, usa path real; para demais, usa extensao dummy com USEFILEATTRIBUTES
        let icon_result = if matches!(extension.as_str(), "exe" | "lnk" | "ico") {
            extract_file_icon_by_path(path, icon_size)
        } else {
            crate::infrastructure::windows::get_file_type_icon(
                false,
                &format!(".{}", extension),
                icon_size,
            )
        };

        match icon_result {
            Ok((rgba_data, width, height)) => {
                let texture = ctx.load_texture(
                    format!("icon_{}", cache_key),
                    egui::ColorImage::from_rgba_unmultiplied(
                        [width as usize, height as usize],
                        &rgba_data,
                    ),
                    egui::TextureOptions::NEAREST,
                );

                let cloned = texture.clone();
                self.cache_manager.icon_cache.put(cache_key, texture);
                Some(cloned)
            }
            Err(_) => None, // Fallback: sem icone
        }
    }

    /// Garante que ÃƒÂ­cone de pasta estÃƒÂ¡ carregado.
    pub fn ensure_folder_icon(&mut self, ctx: &egui::Context) {
        let thumbnail_size = self.thumbnail_size;
        let icon_size = if thumbnail_size < 100.0 {
            IconSize::Small
        } else {
            IconSize::Large
        };

        self.cache_manager
            .ensure_folder_icon(ctx, || windows_infra::extract_folder_icon(icon_size));
    }

    /// Garante que ÃƒÂ­cone de "Este Computador" estÃƒÂ¡ carregado.
    pub fn ensure_computer_icon(&mut self, ctx: &egui::Context) {
        self.cache_manager.ensure_computer_icon(ctx, || {
            windows_infra::extract_computer_icon(IconSize::Small)
        });
    }

    /// Lazily refresh media metadata for the currently selected file.
    pub fn refresh_selected_metadata(&mut self) {
        let current_file = self
            .selected_file
            .as_ref()
            .filter(|f| !f.is_dir)
            .map(|f| f.path.clone());

        match current_file {
            Some(path) => {
                let mtime = std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);

                if let Some((cached_mtime, meta)) = self.metadata_cache.get(&path) {
                    if *cached_mtime == mtime {
                        self.selected_metadata = Some((path, meta.clone()));
                        return;
                    }
                }

                if !self.metadata_loading.contains(&path) {
                    let _ = self.metadata_req_sender.send((path.clone(), mtime));
                    self.metadata_loading.insert(path.clone());
                }

                if !matches!(self.selected_metadata.as_ref(), Some((p, _)) if p == &path) {
                    self.selected_metadata = None;
                }
            }
            None => {
                self.selected_metadata = None;
            }
        }
    }

    fn format_media_duration(ticks_100ns: u64) -> String {
        // 1 tick = 100ns; 10_000_000 ticks = 1s
        let total_seconds = ticks_100ns / 10_000_000;
        let hours = total_seconds / 3600;
        let minutes = (total_seconds % 3600) / 60;
        let seconds = total_seconds % 60;

        if hours > 0 {
            format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
        } else {
            format!("{:02}:{:02}", minutes, seconds)
        }
    }

    fn format_bitrate(bps: u32) -> String {
        let bps = bps as f64;
        if bps >= 1_000_000.0 {
            format!("{:.1} Mbps", bps / 1_000_000.0)
        } else if bps >= 1_000.0 {
            format!("{:.0} Kbps", bps / 1_000.0)
        } else {
            format!("{:.0} bps", bps)
        }
    }

    fn approximate_bitrate(size_bytes: u64, duration_100ns: u64) -> Option<u32> {
        if duration_100ns == 0 {
            return None;
        }
        let seconds = duration_100ns as f64 / 10_000_000.0;
        if seconds <= 0.0 {
            return None;
        }
        let bits_per_sec = (size_bytes as f64 * 8.0) / seconds;
        Some(bits_per_sec.max(0.0) as u32)
    }

    /// Processa mensagens que chegam dos canais de workers
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

                        // 1. Se o path alterado Ã© uma subpasta direta da pasta atual
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

                        // 2. Se o path alterado Ã© UM ARQUIVO dentro de uma subpasta da pasta atual
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

        // 1. STREAMING: Recebe lotes incrementais de FileEntry (Filtrado por geraÃƒÂ§ÃƒÂ£o)
        while let Ok((gen_id, new_batch)) = self.file_entry_receiver.try_recv() {
            if gen_id != self.generation {
                continue; // Descarta dados de uma navegaÃƒÂ§ÃƒÂ£o/refresh anterior
            }

            if new_batch.is_empty() {
                // Lote vazio = Sinal de "Fim do Carregamento" da thread
                self.is_loading_folder = false;
                // OrdenaÃƒÂ§ÃƒÂ£o final para garantir tudo correto
                self.sort_items();
            } else {
                // Chegou dados! Adiciona ÃƒÂ  lista mestre
                self.all_items.extend(new_batch);

                // Reaplica filtro (que jÃ¡ chama sort_items internamente)
                self.filter_items();
            }
            ctx.request_repaint();
        }

        // 2. Cover Worker: Recebe resultados de capas de folder
        let mut folder_updates = false;
        while let Ok((folder_path, cover_opt)) = self.cover_worker_receiver.try_recv() {
            if let Some(cover) = cover_opt {
                // Atualiza em all_items (fonte mutÃ¡vel)
                if let Some(item) = self.all_items.iter_mut().find(|i| i.path == folder_path) {
                    item.folder_cover = Some(cover.clone());
                    self.disk_cache.set_folder_cover(&folder_path, &cover);
                    folder_updates = true;

                    // Requisita thumbnail se necessÃ¡rio (Marcando como em carregamento para evitar loop)
                    if !self.cache_manager.has_thumbnail(&cover)
                        && self.cache_manager.start_loading(cover.clone())
                    {
                        self.request_thumbnail_load(cover);
                    }
                }
            }
        }
        // ReconstrÃ³i items a partir de all_items se houve updates
        if folder_updates {
            self.filter_items();
            ctx.request_repaint();
        }

        // 3. Icon Worker: Recebe resultados de Ã­cones assÃ­ncronos
        while let Ok((path, pixels, width, height)) = self.icon_res_receiver.try_recv() {
            self.loading_icons.remove(&path);

            // Carrega textura no cache de Ã­cones
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
            // --- VALIDAÃ‡ÃƒO DE MEMÃ“RIA ---
            // Se a imagem pertence a uma geraÃ§Ã£o anterior (outra folder), descarta.
            if thumbnail_data.generation != self.generation {
                continue;
            }
            // ----------------------------

            received_any = true;

            // SÃƒÂ³ processa thumbnails (image_data nÃƒÂ£o vazio)
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

    // --- DETALHES (LIST VIEW) ---
    pub fn render_list_view(&mut self, ui: &mut egui::Ui) {
        use crate::ui::views::{list_view, ListViewContext, ListViewOperations};

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

        // Extrair dados necessÃ¡rios para evitar mÃºltiplos borrows
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

        // Criar contexto com referÃªncias mutÃ¡veis separadas
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

        // Usar uma abordagem diferente: coletar aÃ§Ãµes em vetores
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

        // Processar aÃ§Ãµes (bloqueadas durante renomeaÃ§Ã£o)
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

        // Executar aÃ§Ãµes coletadas
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
        use crate::ui::views::{grid_view, GridViewContext, GridViewOperations};

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

        // Extrair dados necessÃ¡rios para evitar mÃºltiplos borrows
        let items = self.items.clone(); // Clone para evitar borrow
        let selected_item = self.selected_item;
        let selected_file = self.selected_file.clone();
        let thumbnail_size = self.thumbnail_size;
        let last_grid_cols = self.last_grid_cols;
        let renaming_state = self.renaming_state.clone();
        let focus_rename = self.focus_rename;
        let folder_icon_texture = self.cache_manager.folder_icon_texture.clone();
        let computer_icon = self.cache_manager.computer_icon.clone();

        // Criar contexto com referÃªncias mutÃ¡veis separadas
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

        // Usar uma abordagem diferente: coletar aÃ§Ãµes em vetores
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

        // Processar aÃ§Ãµes (bloqueadas durante renomeaÃ§Ã£o, exceto clique no prÃ³prio item)
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

        // Executar aÃ§Ãµes coletadas
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

        use crate::ui::components::item_slot::{render_item_slot, ItemSlotContext};

        // Clone item data to avoid borrowing self.items during the render
        let item = self.items[idx].clone();
        let is_renaming = self
            .renaming_state
            .as_ref()
            .map_or(false, |(i, _)| *i == idx);

        // Para evitar conflitos de borrow, coletamos as operaÃ§Ãµes pendentes
        // e executamos depois de renderizar
        let mut pending_thumbnail_loads: Vec<std::path::PathBuf> = Vec::new();
        let mut pending_folder_scans: Vec<std::path::PathBuf> = Vec::new();
        let mut pending_folder_preview_loads: Vec<std::path::PathBuf> = Vec::new();
        let mut pending_rename: Option<usize> = None;

        // Texto de renomeaÃ§Ã£o precisa ser tratado separadamente
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

impl crate::ui::components::item_slot::ItemSlotOperations for ImageViewerApp {
    fn request_thumbnail_load(&mut self, path: std::path::PathBuf) {
        // Call inherent method - uses &self so we need to reborrow
        ImageViewerApp::request_thumbnail_load(&*self, path);
    }

    fn request_folder_scan(&mut self, path: std::path::PathBuf) {
        // Call inherent method - uses &self so we need to reborrow
        ImageViewerApp::request_folder_scan(&*self, path);
    }

    fn request_folder_preview_load(&mut self, path: std::path::PathBuf) {
        self.request_folder_preview_load(path);
    }

    fn rename_item(&mut self, idx: usize) {
        self.rename_with_shell(idx);
    }
}

impl crate::ui::context_menu::ContextMenuOperations for ImageViewerApp {
    fn create_new_folder(&mut self) {
        self.create_new_folder();
    }

    fn command_copy(&mut self, idx: Option<usize>) {
        self.command_copy(idx);
    }

    fn command_cut(&mut self, idx: Option<usize>) {
        self.command_cut(idx);
    }

    fn command_paste(&mut self, idx: Option<usize>) {
        self.command_paste(idx);
    }

    fn rename_item(&mut self, idx: usize) {
        if let Some(item) = self.items.get(idx) {
            self.renaming_state = Some((idx, item.name.clone()));
            self.focus_rename = true;
        }
    }

    fn delete_with_shell(&mut self, idx: Option<usize>) {
        self.delete_with_shell_for_idx(idx);
    }
}

impl ImageViewerApp {
    /// Atualiza o thumbnail persistente do arquivo selecionado de forma que
    /// ele continue visÃ­vel mesmo que o item saia do viewport (e seja removido do cache LRU).
    pub fn update_selected_thumbnail(&mut self) {
        if let Some(selected) = &self.selected_file {
            // Validate path exists before trying to load thumbnail
            if !selected.path.exists() {
                self.selected_file = None;
                self.selected_thumbnail = None;
                return;
            }

            // Tenta pegar do cache. Se nÃ£o estiver lÃ¡, mantÃ©m None (serÃ¡ atualizado via message loop)
            if let Some(tex) = self.cache_manager.texture_cache.peek(&selected.path) {
                self.selected_thumbnail = Some(tex.clone());
            } else {
                // Se mudou de seleÃ§Ã£o e nÃ£o tem no cache, limpa
                self.selected_thumbnail = None;
            }
        } else {
            self.selected_thumbnail = None;
        }
    }

    /// Limpa a seleÃ§Ã£o atual, o thumbnail persistente, metadados e a busca.
    /// Ãštil durante navegaÃ§Ã£o entre pastas.
    pub fn reset_selection_and_search(&mut self) {
        self.selected_item = None;
        self.selected_file = None;
        self.selected_thumbnail = None;
        self.selected_metadata = None;
        self.search_query.clear();
        self.context_menu.target_path = None;
        self.renaming_state = None;
    }

    /// Resolve the target path for a context menu action.
    pub fn context_target_path(&self, item_idx: Option<usize>) -> Option<PathBuf> {
        if let Some(idx) = item_idx {
            return self.items.get(idx).map(|i| i.path.clone());
        }

        if let Some(p) = self.context_menu.target_path.clone() {
            return Some(p);
        }

        if let Some(sel) = &self.selected_file {
            return Some(sel.path.clone());
        }

        Some(PathBuf::from(&self.current_path))
    }

    /// Copy a filesystem path to the Windows clipboard as text.
    pub fn copy_path_to_clipboard(&self, path: &Path) {
        if let Err(e) = file_operations::copy_path_to_clipboard(path) {
            eprintln!("Erro clipboard: {}", e);
        }
    }

    /// Create a Windows shell shortcut (.lnk) pointing to `target` in the same directory.
    pub fn create_shell_shortcut(&self, target: &Path) -> std::result::Result<PathBuf, String> {
        file_operations::create_shortcut(target, &self.current_path)
    }

    pub fn populate_context_menu(
        &mut self,
        ctx: &egui::Context,
        path: &std::path::Path,
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
        if is_empty_area {
            items.push(ContextMenuItem::separator());
            items.push(
                ContextMenuItem::new(-32, "Colar")
                    .with_command("paste")
                    .with_shortcut("Ctrl+V")
                    .enabled(can_paste),
            );
            items.push(ContextMenuItem::new(-1, "Criar pasta").with_shortcut("Ctrl+Shift+N"));
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
            if let Ok(shell_ctx) = extract_shell_menu(hwnd, path) {
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
                        "fixar no acesso rÃ¡pido",
                        "restore previous versions",
                        "restaurar versÃµes anteriores",
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
                        ContextMenuItem::new(-99, "Mostrar mais opÃ§Ãµes")
                            .with_subitems(overflow_shell_items),
                    );
                }

                // Keep the native context alive for command invocation
                self.context_menu.native_context = Some(std::rc::Rc::new(shell_ctx));
            }
        }

        self.context_menu.items = items;
    }
}
