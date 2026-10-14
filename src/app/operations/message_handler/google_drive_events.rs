use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui;

use crate::app::state::ImageViewerApp;
use crate::domain::file_entry::{FileEntry, SyncStatus};
use crate::infrastructure::onedrive;

fn collect_statuses(items: &[FileEntry], ctx: &egui::Context) -> HashMap<PathBuf, SyncStatus> {
    items
        .iter()
        .filter(|item| onedrive::is_google_drive_sync_path(&item.path))
        .map(|item| {
            (
                item.path.clone(),
                onedrive::google_drive_cached_sync_status(&item.path, ctx),
            )
        })
        .collect()
}

fn apply_statuses(
    items: &mut Arc<Vec<FileEntry>>,
    statuses: &HashMap<PathBuf, SyncStatus>,
) -> bool {
    let changed = items.iter().any(|item| {
        statuses
            .get(&item.path)
            .is_some_and(|status| *status != item.sync_status)
    });
    if changed {
        for item in Arc::make_mut(items) {
            if let Some(status) = statuses.get(&item.path) {
                item.sync_status = *status;
            }
        }
    }
    changed
}

impl ImageViewerApp {
    pub(super) fn process_google_drive_statuses(&mut self, ctx: &egui::Context) {
        let statuses = collect_statuses(&self.items, ctx);
        if !statuses.is_empty() {
            apply_statuses(&mut self.all_items, &statuses);
            if apply_statuses(&mut self.items, &statuses) {
                self.items_revision = self.items_revision.wrapping_add(1);
            }
            if let Some(selected) = self.selected_file.as_mut() {
                if let Some(status) = statuses.get(&selected.path) {
                    selected.sync_status = *status;
                }
            }
        }
        if let Some(panel) = self.dual_panel_inactive_state.as_mut() {
            let statuses = collect_statuses(&panel.items, ctx);
            if !statuses.is_empty() {
                apply_statuses(&mut panel.all_items, &statuses);
                if apply_statuses(&mut panel.items, &statuses) {
                    panel.items_revision = panel.items_revision.wrapping_add(1);
                }
            }
        }
    }
}
