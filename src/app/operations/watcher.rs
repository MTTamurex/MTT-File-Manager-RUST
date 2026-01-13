//! File system watcher management
//!
//! This module handles the setup and management of the filesystem watcher
//! to detect external changes in the current directory.

use std::path::{Path, PathBuf};
use notify::{Watcher, RecursiveMode};
use crate::app::state::ImageViewerApp;

impl ImageViewerApp {
    /// Configura o monitoramento da pasta atual
    pub fn watch_current_folder(&mut self) {
        let current_path = self.current_path.clone();

        // Canonicaliza o path para compatibilidade com Windows
        let path_to_watch = if let Ok(p) = Path::new(&current_path).canonicalize() {
            p
        } else {
            PathBuf::from(&current_path)
        };

        // Se o watcher já existe, apenas troca o path monitorado
        if let Some(ref mut _watcher) = self.watcher {
            // Para de monitorar todos os paths antigos (o watcher pode ter múltiplos)
            // Como não temos referência ao path antigo, vamos recriar o watcher
            // (notify não tem API para listar paths monitorados)
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
}
