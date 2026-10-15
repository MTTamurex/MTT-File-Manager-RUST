//! Redirect live panes and every stored tab after a confirmed volume removal.
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::Ordering;

use crate::app::drive_state::normalize_drive_root_key;
use crate::app::dual_panel::PanelSnapshot;
use crate::app::state::ImageViewerApp;
use crate::domain::special_paths::COMPUTER_VIEW_ID;
use crate::tabs::TabState;

pub(super) fn removed_drive_roots(
    previous: &[(String, String)],
    current: &[(String, String)],
    cloud_roots: &HashSet<String>,
) -> Vec<String> {
    let present: HashSet<_> = current
        .iter()
        .map(|(path, _)| path.as_str())
        .chain(cloud_roots.iter().map(String::as_str))
        .filter_map(normalize_drive_root_key)
        .collect();
    previous
        .iter()
        .filter_map(|(path, _)| normalize_drive_root_key(path))
        .filter(|root| !present.contains(root))
        .collect()
}

fn is_on_removed_drive(path: &str, removed: &[String]) -> bool {
    let path = path
        .strip_prefix(r"\\?\")
        .or_else(|| path.strip_prefix(r"\??\"))
        .unwrap_or(path);
    normalize_drive_root_key(path).is_some_and(|root| {
        removed
            .iter()
            .filter_map(|drive| normalize_drive_root_key(drive))
            .any(|drive| drive == root)
    })
}

fn redirect_snapshot(snapshot: &mut PanelSnapshot, removed: &[String]) {
    if !is_on_removed_drive(&snapshot.path, removed) {
        return;
    }
    snapshot
        .navigation
        .navigate_to(COMPUTER_VIEW_ID.to_string());
    snapshot.redirect_to_history_current();
    snapshot.generation = super::folder_loading::allocate_folder_load_generation();
    snapshot
        .folder_load_generation
        .store(snapshot.generation, Ordering::Relaxed);
}

fn redirect_stored_tab(tab: &mut TabState, removed: &[String]) {
    if is_on_removed_drive(&tab.path, removed) {
        tab.navigation.navigate_to(COMPUTER_VIEW_ID.to_string());
        tab.redirect_to_history_current();
        tab.generation = super::folder_loading::allocate_folder_load_generation();
        tab.quick_search_buffer.clear();
    }
    if let Some(snapshot) = &mut tab.dual_panel_inactive_state {
        redirect_snapshot(snapshot, removed);
    }
    if let Some(panels) = &mut tab.dual_panel_tabs {
        for panel_tab in panels
            .left
            .tabs
            .iter_mut()
            .chain(panels.right.tabs.iter_mut())
        {
            redirect_snapshot(&mut panel_tab.snapshot, removed);
        }
    }
}

fn redirect_live_panel(app: &mut ImageViewerApp) {
    app.folder_size_state.cancel_batch();
    app.navigation_state
        .navigation
        .navigate_to(COMPUTER_VIEW_ID.to_string());
    app.miller_columns = Default::default();
    app.loaded_path.clear();
    app.pending_all_items_clear = false;
    app.hold_visible_items_until_load_complete = false;
    app.stale_items_snapshot = None;
    app.setup_computer_view();
}

impl ImageViewerApp {
    pub(crate) fn redirect_tabs_from_removed_drives(&mut self, removed: &[String]) {
        if removed.is_empty() {
            return;
        }
        // Save both live panes before touching their stored tab copies.
        self.sync_to_tab();
        // This is a confirmed removal, not an optimistic eject request. Do not
        // hide the letter persistently: it may already be remounted by the
        // time the next asynchronous discovery result arrives.
        self.drive_state
            .disks
            .retain(|(path, _)| !is_on_removed_drive(path, removed));
        self.drive_state.invalidate_drive_info_refreshes();
        for drive in removed {
            self.drive_state.remove_cached_drive_info(drive);
            self.file_operation_state.mounted_iso_drives.remove(drive);
            for root in [drive.to_ascii_uppercase(), drive.to_ascii_lowercase()] {
                self.directory_cache
                    .invalidate_children(&PathBuf::from(root));
            }
            if let Some(index) = &self.directory_index {
                let _ = index.invalidate_recursive(&PathBuf::from(drive));
            }
        }
        let active_removed = is_on_removed_drive(&self.navigation_state.current_path, removed);
        let inactive_removed = self
            .dual_panel_inactive_state
            .as_ref()
            .is_some_and(|snapshot| is_on_removed_drive(&snapshot.path, removed));
        let inactive_computer = self
            .dual_panel_inactive_state
            .as_ref()
            .is_some_and(|snapshot| snapshot.is_computer_view);
        if inactive_removed && self.dual_panel_enabled {
            self.with_inactive_panel(redirect_live_panel);
        } else if inactive_computer && self.dual_panel_enabled {
            self.with_inactive_panel(|app| app.setup_computer_view());
        } else if let Some(snapshot) = &mut self.dual_panel_inactive_state {
            redirect_snapshot(snapshot, removed);
        }
        let active_computer = self.navigation_state.is_computer_view;
        if active_removed {
            redirect_live_panel(self);
        } else if active_computer {
            self.setup_computer_view();
        }
        if active_removed || inactive_removed {
            self.watch_current_folder();
        }
        if active_removed || inactive_removed || active_computer || inactive_computer {
            self.sync_to_tab();
        }
        for tab in &mut self.tab_manager.tabs {
            redirect_stored_tab(tab, removed);
        }
        self.ui_ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests;
