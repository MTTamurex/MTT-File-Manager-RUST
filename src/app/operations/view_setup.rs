//! View setup: computer view, recycle bin view, drive list
//!
//! This module handles setting up special views like "This PC" and "Recycle Bin".

use std::path::PathBuf;
use std::sync::atomic::Ordering as AtomicOrdering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::app::state::ImageViewerApp;
use crate::domain::file_entry::FileEntry;
use crate::infrastructure::windows as windows_infra;

// PERFORMANCE: Increased from 2s to 30s to avoid periodic HDD access.
// Device insertion/removal is detected instantly via RegisterDeviceNotificationW
// (device_event_receiver in message_handler.rs). This timer is only a safety fallback.
const DRIVE_REFRESH_INTERVAL_MS: u64 = 30000;

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
                            recycle_original_path: Some(item.original_path),
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
        // CRITICAL: Increment generation to invalidate any pending items_rebuild results
        // from the previous folder load. Without this, stale rebuild results (matching
        // the old generation) arrive on the next frame and overwrite our computer view items.
        self.generation += 1;
        self.current_generation
            .store(self.generation, AtomicOrdering::Relaxed);
        self.pending_items_rebuild = false;
        self.pending_items_count = 0;

        // Set computer view
        self.current_path = "Este Computador".to_string();
        self.is_computer_view = true;
        self.is_recycle_bin_view = false;
        self.path_input = "Este Computador".to_string();

        // Load Computer View sort mode
        self.sort_mode = self.sort_mode_computer;

        // Populate items with drives using FAST-ONLY calls (no I/O blocking)
        // detect_drive_type uses GetDriveTypeW which is cached and < 1ms
        // get_volume_info is deferred to background thread
        use crate::domain::file_entry::DriveInfo;

        let mut computer_items = Vec::new();
        for (path, label) in &self.disks {
            let drive_type = windows_infra::detect_drive_type(path);
            let entry = FileEntry {
                path: PathBuf::from(path),
                name: label.clone(),
                is_dir: true,
                size: 0,
                modified: 0,
                folder_cover: None,
                drive_info: Some(DriveInfo {
                    file_system: String::new(),
                    total_space: 0,
                    free_space: 0,
                    drive_type,
                }),
                sync_status: crate::domain::file_entry::SyncStatus::None,
                deletion_date: None,
                recycle_original_path: None,
            };
            computer_items.push(entry);
        }

        self.all_items = computer_items.clone();
        self.items = Arc::new(computer_items);

        // PRE-COMPUTE SECTION INDICES (O(n) once, not per frame)
        self.computer_view_local_indices.clear();
        self.computer_view_network_indices.clear();

        for (i, item) in self.items.iter().enumerate() {
            let is_remote = item.drive_info.as_ref().is_some_and(|di| {
                di.drive_type == crate::infrastructure::windows::DriveType::Remote
            });
            if is_remote {
                self.computer_view_network_indices.push(i);
            } else {
                self.computer_view_local_indices.push(i);
            }
        }

        self.reset_selection_and_search();
        self.total_items = self.disks.len();
        self.is_loading_folder = false;

        // Launch background thread for volume info (total/free space, file_system)
        let disks_snapshot: Vec<String> = self.disks.iter().map(|(p, _)| p.clone()).collect();
        let tx = self.drive_info_tx.clone();
        let ctx = self.ui_ctx.clone();
        std::thread::spawn(move || {
            use crate::infrastructure::windows::get_volume_info;
            let mut results = Vec::new();
            for path in &disks_snapshot {
                let vol = get_volume_info(path);
                let drive_type = crate::infrastructure::windows::detect_drive_type(path);
                results.push((
                    path.clone(),
                    DriveInfo {
                        file_system: vol.file_system,
                        total_space: vol.total_space,
                        free_space: vol.free_space,
                        drive_type,
                    },
                ));
            }
            let _ = tx.send(results);
            ctx.request_repaint();
        });
    }

    /// Launches a background thread to scan drives. Non-blocking.
    pub fn reload_drive_list_async(&mut self) {
        if self.drive_scan_pending {
            return; // Already scanning
        }
        self.drive_scan_pending = true;
        let tx = self.drive_scan_tx.clone();
        let ctx = self.ui_ctx.clone();
        std::thread::spawn(move || {
            let new_disks = crate::infrastructure::windows::get_all_drives();
            let _ = tx.send(new_disks);
            ctx.request_repaint();
        });
    }

    /// Poll for completed background drive scans. Called once per frame.
    pub fn poll_drive_scan(&mut self) {
        if let Ok(new_disks) = self.drive_scan_rx.try_recv() {
            self.drive_scan_pending = false;
            let old_disks = std::mem::take(&mut self.disks);
            let changed = new_disks != old_disks;
            self.disks = new_disks;

            if changed {
                // Invalidate cached drive types since drive list changed
                crate::ui::sidebar::invalidate_drive_type_cache();

                // AUTO-FOCUS PARA ISO RECÉM-MONTADA
                if let Some(_iso_path) = self.pending_iso_mount.take() {
                    let mut target_drive = None;
                    for (new_path, _label) in &self.disks {
                        if !old_disks.iter().any(|(old_path, _)| old_path == new_path)
                            && crate::infrastructure::onedrive::fast_path_exists(
                                std::path::Path::new(new_path),
                            )
                        {
                            target_drive = Some(new_path.clone());
                            break;
                        }
                    }

                    if let Some(drive) = target_drive {
                        self.navigate_to(&drive);
                    } else {
                        self.pending_iso_mount = Some(_iso_path);
                    }
                }

                if self.is_computer_view {
                    self.setup_computer_view();
                }
            }
        }
    }

    pub fn refresh_drives_if_needed(&mut self) {
        if self.last_drive_refresh.elapsed() >= Duration::from_millis(DRIVE_REFRESH_INTERVAL_MS) {
            self.last_drive_refresh = Instant::now();
            self.reload_drive_list_async();
        }
    }

    /// Poll for completed background volume info scans. Called once per frame.
    /// Updates drive_info (total_space, free_space, file_system) in existing items.
    pub fn poll_drive_info(&mut self) {
        if let Ok(results) = self.drive_info_rx.try_recv() {
            if !self.is_computer_view {
                return; // Only update if still in computer view
            }

            // Update all_items with the received drive info
            for item in self.all_items.iter_mut() {
                let item_path = item.path.to_string_lossy().to_string();
                if let Some((_, info)) = results.iter().find(|(p, _)| *p == item_path) {
                    item.drive_info = Some(info.clone());
                }
            }

            // Rebuild Arc<Vec> for items
            self.items = Arc::new(self.all_items.clone());
        }
    }
}
