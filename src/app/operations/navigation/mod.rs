//! Navigation: navigate_to, go_back, go_forward, go_up
//!
//! This module handles history based navigation and switching to special views.

pub mod keyboard;
pub mod selection;

use crate::app::state::ImageViewerApp;

impl ImageViewerApp {
    pub fn navigate_to(&mut self, path: &str) {
        // Normaliza paths de drive roots: garante que "Z:" sempre vire "Z:\"
        // Isso corrige o bug do PathBuf::join não adicionar backslash
        let normalized_path = if path.len() >= 2 && path.chars().nth(1) == Some(':') {
            // É um path Windows com letra de drive
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

        // Se já estamos nesse caminho, não faz nada
        if self.current_path == normalized_path {
            return;
        }

        // Clear loaded_path to allow reload if navigating to same path (for consistency)
        self.loaded_path.clear();

        // Adiciona novo caminho ao histórico
        self.navigation.navigate_to(normalized_path.clone());

        self.current_path = normalized_path.clone();
        self.path_input = normalized_path.clone();
        self.is_computer_view = false;
        self.is_recycle_bin_view = false; // Reset quando navega para qualquer pasta

        // Restore normal folder sort mode
        self.sort_mode = self.sort_mode_normal;

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
                // Invalida preview da pasta que estávamos
                self.cache_manager.invalidate_folder_preview(&previous_path);

                // SYNC TAB STATE
                self.sync_to_tab();

                self.reset_selection_and_search();
                self.setup_computer_view();
            } else if path == "Lixeira" {
                // Invalida preview da pasta que estávamos
                self.cache_manager.invalidate_folder_preview(&previous_path);

                self.reset_selection_and_search();
                self.setup_recycle_bin_view();
            } else {
                let new_path = std::path::PathBuf::from(&path);

                // Se estávamos em uma subpasta do destino, invalida o preview dessa subpasta
                if previous_path.starts_with(&new_path) && previous_path != new_path {
                    self.cache_manager.invalidate_folder_preview(&previous_path);
                }

                self.current_path = path.clone();
                self.loaded_path.clear(); // Clear to allow reload
                self.sync_to_tab();
                self.path_input = self.current_path.clone();
                self.is_computer_view = false;
                self.is_recycle_bin_view = false;
                
                // Restore normal folder sort mode
                self.sort_mode = self.sort_mode_normal;
                
                self.reset_selection_and_search();
                self.watch_current_folder(); // Atualiza o watcher
                self.load_folder(false);
            }
        }
    }

    /// Avança no histórico
    pub fn go_forward(&mut self) {
        if let Some(path) = self.navigation.go_forward().cloned() {
            // Guarda o path atual antes de avançar (para invalidar o preview)
            let previous_path = std::path::PathBuf::from(&self.current_path);

            if path == "Este Computador" {
                // Invalida preview da pasta que estávamos
                self.cache_manager.invalidate_folder_preview(&previous_path);

                // SYNC TAB STATE
                self.sync_to_tab();

                self.reset_selection_and_search();
                self.setup_computer_view();
            } else if path == "Lixeira" {
                // Invalida preview da pasta que estávamos
                self.cache_manager.invalidate_folder_preview(&previous_path);

                self.reset_selection_and_search();
                self.setup_recycle_bin_view();
            } else {
                let new_path = std::path::PathBuf::from(&path);

                // Se estávamos em uma subpasta do destino, invalida o preview dessa subpasta
                if previous_path.starts_with(&new_path) && previous_path != new_path {
                    self.cache_manager.invalidate_folder_preview(&previous_path);
                }

                self.current_path = path.clone();
                self.loaded_path.clear(); // Clear to allow reload
                self.sync_to_tab();
                self.path_input = self.current_path.clone();
                self.is_computer_view = false;
                self.is_recycle_bin_view = false;
                
                // Restore normal folder sort mode
                self.sort_mode = self.sort_mode_normal;
                
                self.reset_selection_and_search();
                self.watch_current_folder();
                self.load_folder(false);
            }
        }
    }

    /// Navega para "Este Computador" view (adicionando ao histórico)
    pub fn navigate_to_computer(&mut self) {
        if self.is_computer_view {
            return;
        }

        self.navigation.navigate_to("Este Computador".to_string());
        // self.sync_to_tab(); // setup_computer_view chama sync_from_tab?? não, a gente sincroniza depois

        self.reset_selection_and_search();
        self.watch_current_folder();
        self.setup_computer_view();
        eprintln!("[COMPUTER-VIEW] navigate_to_computer: after setup, items.len()={}, all_items.len()={}", self.items.len(), self.all_items.len());
        self.sync_to_tab();
        eprintln!("[COMPUTER-VIEW] navigate_to_computer: after sync_to_tab, items.len()={}, all_items.len()={}", self.items.len(), self.all_items.len());
    }

    pub fn navigate_to_recycle_bin(&mut self) {
        if self.is_recycle_bin_view {
            return;
        }

        self.navigation.navigate_to("Lixeira".to_string());
        self.reset_selection_and_search();
        self.watch_current_folder();
        self.setup_recycle_bin_view();
        self.sync_to_tab();
    }

    pub fn go_up_one_level(&mut self) {
        if self.is_computer_view {
            // Já estamos no topo
            return;
        }

        // Se estamos na raiz de um drive (C:\, D:\), subir vai para "Este Computador"
        let parent = std::path::Path::new(&self.current_path).parent();
        if parent.is_none() {
            self.navigate_to_computer();
            return;
        }

        if let Some(parent_path) = parent {
            if parent_path.as_os_str().is_empty() {
                 self.navigate_to_computer();
            } else {
                self.navigate_to(parent_path.to_string_lossy().to_string().as_str());
            }
        } else {
            self.navigate_to_computer();
        }
    }

    /// Pode avançar no histórico?
    pub fn can_go_back(&self) -> bool {
        self.navigation.can_go_back()
    }

    /// Pode avançar no histórico?
    pub fn can_go_forward(&self) -> bool {
        self.navigation.can_go_forward()
    }
}

// Re-export commonly used types from submodules
pub use keyboard::*;
pub use selection::*;
