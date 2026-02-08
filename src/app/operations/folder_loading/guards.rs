use crate::app::state::ImageViewerApp;
use std::sync::atomic::Ordering as AtomicOrdering;
use std::sync::Arc;
use std::time::Instant;

impl ImageViewerApp {
    pub(super) fn should_skip_folder_load(&self, force_refresh: bool) -> bool {
        // GUARD CLAUSE: Prevent spam by checking if we're already on this path
        eprintln!(
            "[GUARD] Checking load_folder: current_path={:?}, loaded_path={:?}, force_refresh={}",
            self.current_path, self.loaded_path, force_refresh
        );

        if !force_refresh && self.current_path == self.loaded_path {
            eprintln!(
                "[GUARD] Skipping load_folder for {:?} - already loaded",
                self.current_path
            );
            return true;
        }

        false
    }

    pub(super) fn mark_folder_load_started(&mut self, force_refresh: bool) {
        eprintln!(
            "[GUARD] load_folder called for {:?} (force_refresh={}, loaded_path={:?})",
            self.current_path, force_refresh, self.loaded_path
        );

        // Mark as loaded immediately to prevent spam.
        self.loaded_path = self.current_path.clone();

        eprintln!(
            "[GUARD] Starting folder loading process for {:?}",
            self.current_path
        );
    }

    pub(super) fn bump_folder_load_generation(&mut self) {
        self.generation += 1; // Incrementa a geração local
        self.current_generation
            .store(self.generation, AtomicOrdering::Relaxed); // Sincroniza com workers
    }

    pub(super) fn reset_folder_loading_state(&mut self, force_refresh: bool) {
        // 1. Limpeza de Estado (UI Thread)
        if force_refresh {
            self.cache_manager.texture_cache.clear();
            self.cache_manager.folder_preview_cache.clear();
            self.cache_manager.failed_thumbnails.clear();
            crate::workers::thumbnail::clear_all_failures();
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
        self.loading_started_at = Instant::now(); // Track loading start for timeout
        self.total_items = 0;
        self.pending_items_rebuild = false;
        self.pending_items_count = 0;
        self.last_items_rebuild = Instant::now();
    }
}
