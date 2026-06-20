use crate::app::state::ImageViewerApp;
use crate::domain::file_entry::FileEntry;
use crate::domain::file_tag::{FileTag, TagColor};
use crate::domain::special_paths::{tag_id_from_view_path, tag_view_path, COMPUTER_VIEW_ID};
use rustc_hash::{FxHashMap, FxHashSet};
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Instant, UNIX_EPOCH};

fn normalize_path_text(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

fn path_is_same_or_descendant(candidate: &Path, root: &Path) -> bool {
    let candidate = normalize_path_text(candidate);
    let root = normalize_path_text(root);
    candidate == root
        || candidate
            .strip_prefix(&root)
            .is_some_and(|suffix| suffix.starts_with('\\'))
}

fn remap_path(candidate: &Path, old_root: &Path, new_root: &Path) -> PathBuf {
    if let Ok(suffix) = candidate.strip_prefix(old_root) {
        return new_root.join(suffix);
    }

    let candidate = candidate.to_string_lossy();
    let old_root = old_root.to_string_lossy();
    let new_root = new_root.to_string_lossy();
    let old_root = old_root.trim_end_matches(['\\', '/']);
    if candidate.len() <= old_root.len() {
        return PathBuf::from(new_root.as_ref());
    }

    let suffix = &candidate[old_root.len()..];
    PathBuf::from(format!(
        "{}{}",
        new_root.trim_end_matches(['\\', '/']),
        suffix
    ))
}

fn tag_sort_key(tag: &FileTag) -> (i64, String) {
    (tag.position, tag.name.to_lowercase())
}

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
    fn recompute_tag_counts_from_assignments(&mut self) {
        let mut counts = FxHashMap::default();
        for tag_ids in self.tag_assignments.values() {
            for tag_id in tag_ids {
                *counts.entry(*tag_id).or_insert(0) += 1;
            }
        }
        self.tag_counts = counts;
    }

    fn refresh_visible_items_after_tag_change(&mut self) {
        self.filter_items();
        self.ui_ctx.request_repaint();
    }

    fn clear_cached_tag_tab_items(tab: &mut crate::tabs::TabState) {
        tab.items = Arc::new(Vec::new());
        tab.all_items = Arc::new(Vec::new());
        tab.items_snapshot_compact = false;
        tab.total_items = 0;
        tab.selected_item = None;
        tab.selected_file = None;
        tab.selected_thumbnail = None;
        tab.selected_metadata = None;
        tab.selected_gif = None;
        tab.multi_selection.clear();
    }

    fn clear_cached_tag_snapshot_items(snapshot: &mut crate::app::dual_panel::PanelSnapshot) {
        snapshot.items = Arc::new(Vec::new());
        snapshot.all_items = Arc::new(Vec::new());
        snapshot.items_snapshot_compact = false;
        snapshot.total_items = 0;
        snapshot.selected_item = None;
        snapshot.selected_file = None;
        snapshot.selected_thumbnail = None;
        snapshot.selected_metadata = None;
        snapshot.selected_gif = None;
        snapshot.multi_selection.clear();
    }

    fn invalidate_cached_tag_views_for_tags(&mut self, tag_ids: &FxHashSet<i64>) {
        if tag_ids.is_empty() {
            return;
        }

        let active_tab = self.tab_manager.active_tab;
        for (index, tab) in self.tab_manager.tabs.iter_mut().enumerate() {
            if index != active_tab
                && tag_id_from_view_path(&tab.path).is_some_and(|id| tag_ids.contains(&id))
            {
                Self::clear_cached_tag_tab_items(tab);
            }

            if let Some(snapshot) = tab.dual_panel_inactive_state.as_mut() {
                if tag_id_from_view_path(&snapshot.path).is_some_and(|id| tag_ids.contains(&id)) {
                    Self::clear_cached_tag_snapshot_items(snapshot);
                }
            }
        }
    }

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
        self.tag_assignments = Arc::new(assignments);
        self.recompute_tag_counts_from_assignments();

        let mut paths: Vec<PathBuf> = self
            .tag_assignments
            .iter()
            .filter(|(_, ids)| ids.contains(&tag_id))
            .map(|(path, _)| path.clone())
            .collect();
        paths.sort_by_key(|path| path.to_string_lossy().to_lowercase());

        let generation = self.generation;
        // Tag views can move between active/inactive dual panels while loading.
        // Use a load-local token so a focus switch does not cancel this worker;
        // stale batches are still discarded by the generation router on receive.
        let current_generation = Arc::new(std::sync::atomic::AtomicUsize::new(generation));
        let sender = self.file_entry_sender.clone();
        let ui_ctx = self.ui_ctx.clone();
        let show_hidden = self.show_hidden_files;

        std::thread::Builder::new()
            .name("tag-view-load".into())
            .spawn(move || {
                const BATCH_SIZE: usize = 100;
                let mut batch = Vec::with_capacity(BATCH_SIZE);

                for path in paths {
                    if current_generation.load(std::sync::atomic::Ordering::Relaxed) != generation {
                        return;
                    }
                    if let Some(entry) = tag_view_file_entry(path, show_hidden) {
                        batch.push(entry);
                        if batch.len() >= BATCH_SIZE {
                            let _ = sender.send((generation, std::mem::take(&mut batch)));
                            ui_ctx.request_repaint();
                            batch = Vec::with_capacity(BATCH_SIZE);
                        }
                    }
                }

                if current_generation.load(std::sync::atomic::Ordering::Relaxed) == generation {
                    if !batch.is_empty() {
                        let _ = sender.send((generation, batch));
                    }
                    let _ = sender.send((generation, Vec::new()));
                    ui_ctx.request_repaint();
                }
            })
            .ok();
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

    pub fn assign_tag_to_paths(&mut self, paths: &[PathBuf], tag_id: i64) {
        if paths.is_empty() || !self.tag_definitions.contains_key(&tag_id) {
            return;
        }

        let mut changed = false;
        for path in paths {
            let already_assigned = self
                .tag_assignments
                .get(path)
                .is_some_and(|ids| ids.contains(&tag_id));
            if already_assigned {
                continue;
            }

            if !self.app_state_db.assign_tag(path, tag_id) {
                continue;
            }

            let assignments = Arc::make_mut(&mut self.tag_assignments);
            assignments.entry(path.clone()).or_default().push(tag_id);
            *self.tag_counts.entry(tag_id).or_insert(0) += 1;
            changed = true;
        }

        if changed {
            let mut changed_tags = FxHashSet::default();
            changed_tags.insert(tag_id);
            self.invalidate_cached_tag_views_for_tags(&changed_tags);
            self.refresh_visible_items_after_tag_change();
        }
    }

    pub fn unassign_tag_from_paths(&mut self, paths: &[PathBuf], tag_id: i64) {
        if paths.is_empty() {
            return;
        }

        let mut changed = false;
        for path in paths {
            let has_tag = self
                .tag_assignments
                .get(path)
                .is_some_and(|ids| ids.contains(&tag_id));
            if !has_tag {
                continue;
            }

            if !self.app_state_db.unassign_tag(path, tag_id) {
                continue;
            }

            let assignments = Arc::make_mut(&mut self.tag_assignments);
            if let Some(ids) = assignments.get_mut(path) {
                ids.retain(|id| *id != tag_id);
                if ids.is_empty() {
                    assignments.remove(path);
                }
            }
            if let Some(count) = self.tag_counts.get_mut(&tag_id) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    self.tag_counts.remove(&tag_id);
                }
            }
            changed = true;
        }

        if changed {
            let mut changed_tags = FxHashSet::default();
            changed_tags.insert(tag_id);
            self.invalidate_cached_tag_views_for_tags(&changed_tags);
            self.refresh_visible_items_after_tag_change();
        }
    }

    pub fn toggle_tag_on_paths(&mut self, paths: &[PathBuf], tag_id: i64) {
        if self.paths_have_tag(paths, tag_id) {
            self.unassign_tag_from_paths(paths, tag_id);
        } else {
            self.assign_tag_to_paths(paths, tag_id);
        }
    }

    pub fn create_new_tag(&mut self, name: &str, color: TagColor) -> Option<i64> {
        let id = self.app_state_db.create_tag(name, color)?;
        let position = self
            .tag_definitions
            .values()
            .map(|tag| tag.position)
            .max()
            .unwrap_or(-1)
            + 1;
        self.tag_definitions.insert(
            id,
            FileTag {
                id,
                name: name.trim().to_string(),
                color,
                position,
            },
        );
        self.ui_ctx.request_repaint();
        Some(id)
    }

    pub fn rename_tag_definition(&mut self, tag_id: i64, name: &str) -> bool {
        if !self.app_state_db.rename_tag(tag_id, name) {
            return false;
        }
        if let Some(tag) = self.tag_definitions.get_mut(&tag_id) {
            tag.name = name.trim().to_string();
        }
        self.ui_ctx.request_repaint();
        true
    }

    pub fn update_tag_definition_color(&mut self, tag_id: i64, color: TagColor) -> bool {
        if !self.app_state_db.update_tag_color(tag_id, color) {
            return false;
        }
        if let Some(tag) = self.tag_definitions.get_mut(&tag_id) {
            tag.color = color;
        }
        self.ui_ctx.request_repaint();
        true
    }

    pub fn delete_tag_definition(&mut self, tag_id: i64) -> bool {
        if !self.app_state_db.delete_tag(tag_id) {
            return false;
        }

        self.tag_definitions.remove(&tag_id);
        self.tag_counts.remove(&tag_id);
        let assignments = Arc::make_mut(&mut self.tag_assignments);
        assignments.retain(|_, ids| {
            ids.retain(|id| *id != tag_id);
            !ids.is_empty()
        });

        if self.active_tag_filter == Some(tag_id) {
            self.active_tag_filter = None;
            self.save_preferences();
            if tag_id_from_view_path(&self.navigation_state.current_path) == Some(tag_id) {
                self.navigate_to_computer();
            }
        }

        self.refresh_visible_items_after_tag_change();
        true
    }

    pub fn set_tag_filter(&mut self, tag_id: Option<i64>) {
        let tag_id = tag_id.filter(|id| self.tag_definitions.contains_key(id));
        if let Some(tag_id) = tag_id {
            let view_path = tag_view_path(tag_id);
            if self.navigation_state.current_path != view_path {
                self.navigation_state.navigation.navigate_to(view_path);
            }
            self.setup_tag_view(tag_id);
            self.save_preferences();
            self.sync_to_tab();
            return;
        }

        let in_tag_view = tag_id_from_view_path(&self.navigation_state.current_path).is_some();
        if self.active_tag_filter.is_none() && !in_tag_view {
            return;
        }
        self.active_tag_filter = None;
        self.save_preferences();

        if in_tag_view {
            let fallback = self
                .navigation_state
                .navigation
                .paths
                .iter()
                .take(self.navigation_state.navigation.current_index)
                .rev()
                .find(|path| tag_id_from_view_path(path).is_none())
                .cloned()
                .unwrap_or_else(|| COMPUTER_VIEW_ID.to_string());
            if fallback == COMPUTER_VIEW_ID {
                self.navigate_to_computer();
            } else if fallback == crate::domain::special_paths::RECYCLE_BIN_VIEW_ID {
                self.navigate_to_recycle_bin();
            } else {
                self.navigate_to(&fallback);
            }
        } else {
            self.refresh_visible_items_after_tag_change();
        }
    }

    pub fn paths_have_tag(&self, paths: &[PathBuf], tag_id: i64) -> bool {
        !paths.is_empty()
            && paths.iter().all(|path| {
                self.tag_assignments
                    .get(path)
                    .is_some_and(|ids| ids.contains(&tag_id))
            })
    }

    pub fn paths_tag_ids(&self, paths: &[PathBuf]) -> Vec<i64> {
        let mut seen = FxHashSet::default();
        for path in paths {
            if let Some(ids) = self.tag_assignments.get(path) {
                for id in ids {
                    seen.insert(*id);
                }
            }
        }

        let mut ids: Vec<i64> = seen.into_iter().collect();
        ids.sort_by_key(|id| {
            self.tag_definitions
                .get(id)
                .map(tag_sort_key)
                .unwrap_or((i64::MAX, String::new()))
        });
        ids
    }

    pub fn clear_tag_assignments_for_paths(&mut self, paths: &[PathBuf]) {
        if paths.is_empty() {
            return;
        }

        let changed_tags: FxHashSet<i64> = self
            .tag_assignments
            .iter()
            .filter(|(assigned_path, _)| {
                paths
                    .iter()
                    .any(|path| path_is_same_or_descendant(assigned_path, path))
            })
            .flat_map(|(_, ids)| ids.iter().copied())
            .collect();

        self.app_state_db.clear_tag_assignments_for_paths(paths);
        let assignments = Arc::make_mut(&mut self.tag_assignments);
        let before_len = assignments.len();
        assignments.retain(|assigned_path, _| {
            !paths
                .iter()
                .any(|path| path_is_same_or_descendant(assigned_path, path))
        });
        let changed = assignments.len() != before_len;
        if changed {
            self.recompute_tag_counts_from_assignments();
            self.invalidate_cached_tag_views_for_tags(&changed_tags);
            self.refresh_visible_items_after_tag_change();
        }
    }

    pub fn move_tag_assignments_for_path(&mut self, old_path: &Path, new_path: &Path) {
        if old_path == new_path {
            return;
        }

        let moved_assignments: Vec<(PathBuf, Vec<i64>)> = self
            .tag_assignments
            .iter()
            .filter(|(path, _)| path_is_same_or_descendant(path, old_path))
            .map(|(path, ids)| (path.clone(), ids.clone()))
            .collect();
        if moved_assignments.is_empty() {
            return;
        }

        let changed_tags: FxHashSet<i64> = moved_assignments
            .iter()
            .flat_map(|(_, ids)| ids.iter().copied())
            .collect();

        if !self.app_state_db.move_tag_assignments(old_path, new_path) {
            return;
        }

        let remap_entry = |entry: &mut FileEntry| {
            if path_is_same_or_descendant(&entry.path, old_path) {
                entry.path = remap_path(&entry.path, old_path, new_path);
                if let Some(name) = entry.path.file_name() {
                    entry.name = name.to_string_lossy().to_string();
                }
            }
            if let Some(cover) = entry.folder_cover.as_mut() {
                if path_is_same_or_descendant(cover, old_path) {
                    *cover = remap_path(cover, old_path, new_path);
                }
            }
        };
        let items_diverged = !Arc::ptr_eq(&self.items, &self.all_items);
        for entry in Arc::make_mut(&mut self.all_items).iter_mut() {
            remap_entry(entry);
        }
        if items_diverged {
            for entry in Arc::make_mut(&mut self.items).iter_mut() {
                remap_entry(entry);
            }
        } else {
            self.share_visible_items_from_all_items();
        }

        if let Some(snapshot) = self.dual_panel_inactive_state.as_mut() {
            for entry in Arc::make_mut(&mut snapshot.all_items).iter_mut() {
                remap_entry(entry);
            }
            if !snapshot.items_snapshot_compact {
                for entry in Arc::make_mut(&mut snapshot.items).iter_mut() {
                    remap_entry(entry);
                }
            }
        }

        let assignments = Arc::make_mut(&mut self.tag_assignments);
        for (old_assigned_path, ids) in moved_assignments {
            assignments.remove(&old_assigned_path);
            let new_assigned_path = remap_path(&old_assigned_path, old_path, new_path);
            let target = assignments.entry(new_assigned_path).or_default();
            for id in ids {
                if !target.contains(&id) {
                    target.push(id);
                }
            }
        }
        self.recompute_tag_counts_from_assignments();
        self.invalidate_cached_tag_views_for_tags(&changed_tags);
        self.refresh_visible_items_after_tag_change();
    }
}
