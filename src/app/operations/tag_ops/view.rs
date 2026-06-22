use super::*;
use crate::app::state::{FolderLoadError, ImageViewerApp};
use crate::domain::file_entry::FileEntry;
use crate::domain::file_tag::FileTag;
use crate::domain::special_paths::{tag_id_from_view_path, tag_view_path};
use std::os::windows::fs::MetadataExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Instant, UNIX_EPOCH};

fn tag_view_file_entry(path: PathBuf, show_hidden: bool) -> Option<FileEntry> {
    let metadata = std::fs::metadata(&path).ok()?;
    let name = path.file_name()?.to_string_lossy().to_string();
    let is_archive = crate::domain::file_entry::is_archive_extension(&name);
    let is_real_dir = metadata.is_dir();
    let is_dir = is_real_dir || is_archive;
    let is_hidden = (metadata.file_attributes() & 0x2) != 0;
    if is_hidden && !show_hidden {
        return None;
    }

    let size = if is_real_dir && !is_archive {
        0
    } else {
        metadata.len()
    };
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let created = metadata
        .created()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .filter(|created| *created > 0);
    let sync_status = crate::infrastructure::onedrive::sync_status_for_path(&path)
        .unwrap_or(crate::domain::file_entry::SyncStatus::None);

    Some(FileEntry {
        path,
        name,
        is_dir,
        size,
        modified,
        created,
        folder_cover: None,
        drive_info: None,
        sync_status,
        is_hidden,
        recycle_bin: None,
    })
}

impl ImageViewerApp {
    pub fn sorted_tag_definitions(&self) -> Vec<FileTag> {
        let mut tags: Vec<FileTag> = self.tag_definitions.values().cloned().collect();
        tags.sort_by_key(tag_sort_key);
        tags
    }

    pub fn tag_view_display_name(&self, tag_id: i64) -> String {
        let name = self
            .tag_definitions
            .get(&tag_id)
            .map(|tag| tag.name.clone())
            .unwrap_or_else(|| tag_id.to_string());
        rust_i18n::t!("tags.filter_active", name = name).to_string()
    }

    pub fn tag_view_display_name_for_path(&self, path: &str) -> Option<String> {
        tag_id_from_view_path(path).map(|tag_id| self.tag_view_display_name(tag_id))
    }

    pub fn setup_tag_view(&mut self, tag_id: i64) {
        if !self.tag_definitions.contains_key(&tag_id) {
            self.active_tag_filter = None;
            self.navigate_to_computer();
            return;
        }

        let view_path = tag_view_path(tag_id);
        self.bump_folder_load_generation();
        self.invalidate_active_items_rebuild();

        self.navigation_state.current_path = view_path.clone();
        self.navigation_state.path_input = self.tag_view_display_name(tag_id);
        self.navigation_state.is_computer_view = false;
        self.navigation_state.is_recycle_bin_view = false;
        self.active_tag_filter = Some(tag_id);

        self.sort_mode = self.sort_mode_normal;
        self.sort_descending = self.sort_descending_normal;
        self.folders_position = self.folders_position_normal;
        self.current_folder_locked = false;

        self.items = Arc::new(Vec::new());
        self.all_items_mut().clear();
        self.total_items = 0;
        self.is_loading_folder = true;
        self.folder_load_error = None;
        self.pending_all_items_clear = false;
        self.hold_visible_items_until_load_complete = false;
        self.pending_items_rebuild = false;
        self.pending_items_count = 0;
        self.loading_started_at = Instant::now();
        self.loaded_path = view_path;
        self.reset_selection_and_search();

        let assignments = self.app_state_db.get_all_tag_assignments();
        self.set_tag_assignments(assignments);
        self.recompute_tag_counts_from_assignments();

        let mut paths: Vec<PathBuf> = self
            .tag_assignments
            .iter()
            .filter(|(_, ids)| ids.contains(&tag_id))
            .map(|(path, _)| path.clone())
            .collect();
        paths.sort_by_key(|path| path.to_string_lossy().to_lowercase());

        let generation = self.generation;
        let sender = self.file_entry_sender.clone();
        let ui_ctx = self.ui_ctx.clone();
        let show_hidden = self.show_hidden_files;
        let failure_path = PathBuf::from(&self.navigation_state.current_path);

        let spawn_result = std::thread::Builder::new()
            .name("tag-view-load".into())
            .spawn(move || {
                const BATCH_SIZE: usize = 100;
                let mut batch = Vec::with_capacity(BATCH_SIZE);

                for path in paths {
                    if let Some(entry) = tag_view_file_entry(path, show_hidden) {
                        batch.push(entry);
                        if batch.len() >= BATCH_SIZE {
                            let _ = sender.send((generation, std::mem::take(&mut batch)));
                            ui_ctx.request_repaint();
                            batch = Vec::with_capacity(BATCH_SIZE);
                        }
                    }
                }

                if !batch.is_empty() {
                    let _ = sender.send((generation, batch));
                }
                let _ = sender.send((generation, Vec::new()));
                ui_ctx.request_repaint();
            });

        if let Err(error) = spawn_result {
            let message = format!("Failed to spawn tag view loader: {}", error);
            log::error!("[TAGS] {}", message);
            self.is_loading_folder = false;
            self.folder_load_error = Some(FolderLoadError::other(
                failure_path.clone(),
                message.clone(),
            ));
            let _ = self
                .folder_load_failure_sender
                .send((generation, FolderLoadError::other(failure_path, message)));
            self.ui_ctx.request_repaint();
        }
    }

    pub fn reload_visible_tag_views(&mut self) -> bool {
        let active_tag_id = tag_id_from_view_path(&self.navigation_state.current_path);
        let inactive_tag_id = self
            .dual_panel_inactive_state
            .as_ref()
            .and_then(|snapshot| tag_id_from_view_path(&snapshot.path));

        if active_tag_id.is_none() && inactive_tag_id.is_none() {
            return false;
        }

        if let Some(tag_id) = active_tag_id {
            self.setup_tag_view(tag_id);
        }

        if let Some(tag_id) = inactive_tag_id {
            self.with_inactive_panel(|app| {
                app.setup_tag_view(tag_id);
            });
        }

        true
    }
}
