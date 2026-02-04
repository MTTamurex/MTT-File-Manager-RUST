//! File system watcher management
//!
//! This module handles the setup and management of the filesystem watcher
//! to detect external changes in the current directory.

use std::path::{Path, PathBuf};
#[cfg(feature = "notify-watcher")]
use notify::{RecursiveMode, Watcher};
use crate::app::state::ImageViewerApp;

impl ImageViewerApp {
    /// Configura o monitoramento da pasta atual
    ///
    /// USO DUAL:
    /// 1. Novo: Drive-wide watcher (monitora drive inteiro, filtra por prefixo)
    /// 2. Legacy: notify-watcher (monitora pasta específica)
    ///
    /// O drive watcher é mais eficiente para navegação rápida pois não precisa
    /// recriar o watcher a cada mudança de pasta no mesmo drive.
    pub fn watch_current_folder(&mut self) {
        let current_path = self.current_path.clone();
        eprintln!("[WATCHER] Setting up for: {}", current_path);
        
        // NOVO: Ativa drive-wide watcher (File Pilot optimization)
        // Isso monitora o drive inteiro e filtra eventos pelo prefixo atual
        let path_buf = PathBuf::from(&current_path);
        eprintln!("[WATCHER] Calling drive_watcher.watch_path({:?})", path_buf);
        self.drive_watcher.watch_path(path_buf);
        eprintln!("[WATCHER] drive_watcher active drives: {:?}",
            if self.drive_watcher.is_active() { "active" } else { "inactive" });
        
        // LEGACY: Mantém notify-watcher como fallback para compatibilidade
        #[cfg(feature = "notify-watcher")]
        self.setup_notify_watcher();
    }
    
    /// Setup legacy notify-based watcher (fallback)
    #[cfg(feature = "notify-watcher")]
    fn setup_notify_watcher(&mut self) {
        let current_path = self.current_path.clone();

        // Canonicaliza o path para compatibilidade com Windows
        let path_to_watch = if let Ok(p) = Path::new(&current_path).canonicalize() {
            eprintln!("[NOTIFY-WATCHER] Canonicalized path: {:?}", p);
            p
        } else {
            eprintln!("[NOTIFY-WATCHER] Using original path (canonicalize failed)");
            PathBuf::from(&current_path)
        };

        // Drop o watcher anterior se existir
        if self.watcher.is_some() {
            eprintln!("[NOTIFY-WATCHER] Dropping previous watcher");
            self.watcher = None;
        }

        // Cria ou recria o watcher
        let tx = self.fs_event_sender.clone();
        let ctx_clone = self.ui_ctx.clone();

        let watcher_result =
            notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                match &res {
                    Ok(event) => {
                        eprintln!("[NOTIFY-WATCHER] Event received: kind={:?}, paths={:?}",
                            event.kind, event.paths);
                    }
                    Err(e) => {
                        eprintln!("[NOTIFY-WATCHER] Event error: {}", e);
                    }
                }
                let _ = tx.send(res);
                ctx_clone.request_repaint();
            });

        match watcher_result {
            Ok(mut watcher) => {
                match watcher.watch(&path_to_watch, RecursiveMode::NonRecursive) {
                    Ok(_) => {
                        eprintln!("[NOTIFY-WATCHER] Successfully watching: {:?}", path_to_watch);
                        self.watcher = Some(watcher);
                    }
                    Err(e) => {
                        eprintln!("[NOTIFY-WATCHER] Failed to watch path: {:?} - Error: {}", path_to_watch, e);
                    }
                }
            }
            Err(e) => {
                eprintln!("[NOTIFY-WATCHER] Failed to create watcher: {}", e);
            }
        }
    }
}
