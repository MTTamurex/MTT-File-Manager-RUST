//! Partial removals update memory; only terminal events run full refreshes.
use crate::app::state::ImageViewerApp;
use crate::infrastructure::diagnostic_logger::navigation_trace::Stage;
use std::path::PathBuf;

#[cfg(feature = "notify-watcher")]
fn absorb_remove_event(
    activity: &crate::infrastructure::delete_engine::activity::DeleteActivity,
    event: &mut crate::app::state::TimestampedNotifyEvent,
) -> bool {
    if let Ok(notification) = &mut event.result {
        if matches!(notification.kind, notify::EventKind::Remove(_)) {
            notification.paths.retain(|path| !activity.contains(path));
            return notification.paths.is_empty() && !event.overflow;
        }
    }
    false
}

impl ImageViewerApp {
    #[cfg(feature = "notify-watcher")]
    pub(super) fn absorb_engine_remove_event(
        &self,
        event: &mut crate::app::state::TimestampedNotifyEvent,
    ) -> bool {
        absorb_remove_event(
            &self.file_operation_state.copy_runtime.delete_activity,
            event,
        )
    }

    pub(super) fn handle_delete_progress(
        &mut self,
        parent_folders: Vec<PathBuf>,
        deleted_paths: Vec<PathBuf>,
        current_path_norm: &str,
    ) {
        let mut timing = Stage::new("delete_progress_apply");
        self.apply_deleted_paths(&parent_folders, deleted_paths, current_path_norm);
        timing.lap("delete_progress_parent_snapshots");
        self.invalidate_deleted_parent_listings(&parent_folders, false);
    }

    fn invalidate_deleted_parent_listings(&mut self, parents: &[PathBuf], refresh_covers: bool) {
        self.invalidate_panel_tab_snapshots_for_paths(parents);
        let _timing = Stage::new("delete_progress_parent_index_queue");
        for parent in parents {
            self.directory_dirty_registry.mark_dirty(parent);
            self.directory_cache.invalidate(parent);
            self.miller_columns.invalidate(parent);
            if let Some(panel) = &mut self.dual_panel_inactive_state {
                panel.miller_columns.invalidate(parent);
            }
            self.sidebar_tree.clear_children(parent);
            if refresh_covers {
                self.invalidate_folder_cover_state(parent);
            }
            self.clear_tab_cache_for_normalized_path(&Self::normalize_for_match(parent));
        }
        if let Some(index) = &self.directory_index {
            if !index.invalidate_later(parents.iter().map(|path| (path.clone(), false)).collect()) {
                log::warn!("[DELETE] Could not queue persistent listing cleanup");
            }
        }
    }

    pub(super) fn handle_delete_completed(
        &mut self,
        parent_folders: Vec<PathBuf>,
        deleted_paths: Vec<PathBuf>,
        current_path_norm: &str,
    ) {
        let current_folder_deleted =
            self.apply_deleted_paths(&parent_folders, deleted_paths, current_path_norm);
        self.invalidate_deleted_parent_listings(&parent_folders, true);
        let affected: Vec<_> = parent_folders.iter().collect();
        self.reload_inactive_panel_if_matches(&affected);
        if !current_folder_deleted
            && parent_folders
                .iter()
                .any(|parent| Self::normalize_for_match(parent) == current_path_norm)
        {
            self.loaded_path.clear();
            self.reload_current_folder_preserving_icon_cache();
        }
    }

    fn apply_deleted_paths(
        &mut self,
        parent_folders: &[PathBuf],
        deleted_paths: Vec<PathBuf>,
        current_path_norm: &str,
    ) -> bool {
        let mut timing = Stage::new("delete_sizes");
        self.invalidate_content_ancestor_sizes(parent_folders);
        for parent in parent_folders {
            self.invalidate_folder_size_cache(parent);
        }
        timing.lap("delete_navigation_history");
        let current_folder_deleted =
            self.remove_deleted_paths_from_navigation_history(&deleted_paths);
        timing.lap("delete_tab_snapshots");
        self.invalidate_panel_tab_snapshots_for_paths(&deleted_paths);
        timing.lap("delete_directory_lookup");
        let affects_current_listing = parent_folders
            .iter()
            .any(|parent| Self::normalize_for_match(parent) == current_path_norm);
        let known_directories: std::collections::HashSet<_> = if affects_current_listing {
            self.all_items
                .iter()
                .chain(self.items.iter())
                .chain(self.selected_file.iter())
                .filter(|item| item.is_dir)
                .map(|item| Self::normalize_for_match(&item.path))
                .collect()
        } else {
            Default::default()
        };
        timing.lap("delete_item_caches_and_index");
        let mut index_invalidations = Vec::new();
        for path in &deleted_paths {
            let path_norm = Self::normalize_for_match(path);
            if known_directories.contains(&path_norm)
                || self.has_panel_tab_snapshot_for_path(path)
                || path_norm == current_path_norm
            {
                self.directory_cache.invalidate_children(path);
                self.directory_dirty_registry.mark_dirty(path);
                index_invalidations.push((path.clone(), true));
                self.miller_columns.invalidate(path);
            }
            self.cache_manager.texture_cache.pop(path);
            self.cache_manager.loading_set.remove(path);
            self.cache_manager.pop_rgba_data(path);
            self.cache_manager.failed_thumbnails.pop(path);
            self.multi_selection.remove(path);
            if self.file_operation_state.pending_deletions.len() < 10_000 {
                self.file_operation_state
                    .pending_deletions
                    .insert(path.clone(), ());
            }
        }
        if let Some(index) = &self.directory_index {
            if !index.invalidate_later(index_invalidations) {
                log::warn!("[DELETE] Could not queue removed-directory cleanup");
            }
        }
        if self
            .selected_file
            .as_ref()
            .is_some_and(|file| deleted_paths.contains(&file.path))
        {
            self.selected_item = None;
            self.selected_file = None;
        }
        timing.lap("delete_active_listing_rebuild");
        if affects_current_listing {
            self.remove_deleted_paths_from_listing(&deleted_paths);
        }
        timing.lap("delete_inactive_listing_rebuild");
        let affects_inactive_listing =
            self.dual_panel_inactive_state
                .as_ref()
                .is_some_and(|snapshot| {
                    parent_folders.iter().any(|parent| {
                        Self::normalize_for_match(parent)
                            == Self::normalize_for_match(std::path::Path::new(&snapshot.path))
                    })
                });
        if affects_inactive_listing {
            self.with_inactive_panel(|app| {
                app.remove_deleted_paths_from_listing(&deleted_paths);
            });
        }
        timing.lap("delete_inactive_navigation");
        self.navigate_inactive_panel_after_deleted_paths(&deleted_paths);
        timing.lap("delete_tag_cleanup");
        self.clear_tag_assignments_for_paths(&deleted_paths);
        timing.lap("delete_disk_cache_queue");
        self.enqueue_disk_cache_invalidations_forced(deleted_paths);
        timing.lap("delete_redirect_navigation");
        if current_folder_deleted {
            self.navigate_to_nearest_valid_ancestor();
        }
        current_folder_deleted
    }
}

#[cfg(all(test, feature = "notify-watcher"))]
mod tests {
    use super::*;
    use crate::app::state::TimestampedNotifyEvent;
    use crate::infrastructure::delete_engine::activity::DeleteActivity;
    use std::sync::Arc;
    use std::time::Instant;

    #[test]
    fn deletion_absorbs_only_owned_removals_preserving_unrelated_events_and_overflow() {
        let activity = Arc::new(DeleteActivity::default());
        let _deletion = activity.begin(&[PathBuf::from(r"C:\Data\Folder")]);
        let make_event = || TimestampedNotifyEvent {
            received_at: Instant::now(),
            result: Ok(notify::Event::new(notify::EventKind::Remove(
                notify::event::RemoveKind::File,
            ))
            .add_path(PathBuf::from(r"C:\Data\Folder\file.txt"))),
            overflow: false,
        };
        assert!(absorb_remove_event(&activity, &mut make_event()));
        let mut mixed = make_event();
        mixed
            .result
            .as_mut()
            .unwrap()
            .paths
            .push(PathBuf::from(r"C:\Other\file.txt"));
        assert!(!absorb_remove_event(&activity, &mut mixed));
        assert_eq!(
            mixed.result.unwrap().paths,
            vec![PathBuf::from(r"C:\Other\file.txt")]
        );
        let mut overflow = make_event();
        overflow.overflow = true;
        assert!(!absorb_remove_event(&activity, &mut overflow));
        let mut created = make_event();
        created.result.as_mut().unwrap().kind =
            notify::EventKind::Create(notify::event::CreateKind::File);
        assert!(!absorb_remove_event(&activity, &mut created));
        assert_eq!(created.result.unwrap().paths.len(), 1);
    }
}
