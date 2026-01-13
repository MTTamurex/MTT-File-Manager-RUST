//! View setup: computer view, recycle bin view, drive list
//!
//! This module handles setting up special views like "This PC" and "Recycle Bin".

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering as AtomicOrdering;
use std::time::{Duration, Instant};

use crate::app::state::ImageViewerApp;
use crate::infrastructure::windows as windows_infra;
use crate::domain::file_entry::FileEntry;

const DRIVE_REFRESH_INTERVAL_MS: u64 = 2000;

impl ImageViewerApp {
    pub fn setup_recycle_bin_view(&mut self) {
        self.current_path = "Lixeira".to_string();
        self.is_computer_view = false;
        self.is_recycle_bin_view = true;
        self.path_input = "Lixeira".to_string();
        self.is_loading_folder = true;
        self.items = Arc::new(Vec::new());
        self.all_items.clear();
        self.total_items = 0;

        // Incrementa geração para invalidar thumbnails antigos
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
                        // Verifica se a geração ainda é válida (cancelamento rápido)
                        if gen_clone.load(AtomicOrdering::Relaxed) != my_gen {
                            return;
                        }

                        // Cria um path "virtual" baseado na extensão para carregar ícone correto
                        // O path real não existe mais, mas o ícone é baseado na extensão
                        // O path real ($R) é necessário para ler a data de exclusão ($I creation time)
                        // Se physical_path estiver vazio (falha ao ler), usamos a lógica antiga de dummy.
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
                            path: file_path, // Path físico ($R) para permitir get_deletion_date
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

    /// Configura a visão de "Este Computador" sem afetar o histórico
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
}
