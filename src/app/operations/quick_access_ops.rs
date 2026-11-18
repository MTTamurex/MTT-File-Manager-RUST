use crate::app::ImageViewerApp;
use crate::domain::quick_access_item::{
    move_quick_access_item, ordered_quick_access_items, QuickAccessItem,
};

impl ImageViewerApp {
    pub(crate) fn reorder_quick_access_item(&mut self, from: usize, to: usize) {
        let mut order = ordered_quick_access_items(&self.pinned_folders, &self.folder_groups);
        if !move_quick_access_item(&mut order, from, to) {
            return;
        }

        self.persist_quick_access_order(&order);
    }

    pub(crate) fn persist_quick_access_order(&mut self, order: &[QuickAccessItem]) {
        if let Err(error) = self.app_state_db.update_quick_access_positions(order) {
            log::error!("[QUICK-ACCESS] Failed to persist item order: {error}");
            return;
        }

        for (position, item) in order.iter().enumerate() {
            match item {
                QuickAccessItem::PinnedFolder(path) => {
                    if let Some(folder) = self
                        .pinned_folders
                        .iter_mut()
                        .find(|folder| folder.path == *path)
                    {
                        folder.position = position as i64;
                    }
                }
                QuickAccessItem::FolderGroup(group_id) => {
                    if let Some(group) = self
                        .folder_groups
                        .iter_mut()
                        .find(|group| group.id == *group_id)
                    {
                        group.position = position as i64;
                    }
                }
            }
        }
        self.pinned_folders.sort_by_key(|folder| folder.position);
        self.folder_groups
            .sort_by_key(|group| (group.position, group.id));
    }
}
