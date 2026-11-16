//! Replace discarded listings without copy-on-write cloning shared tab data.

use crate::app::state::ImageViewerApp;
use crate::application::grouping::GroupProjection;
use crate::domain::file_entry::FileEntry;
use std::sync::Arc;

fn clear_listing(
    items: &mut Arc<Vec<FileEntry>>,
    all_items: &mut Arc<Vec<FileEntry>>,
    projection: &mut Arc<GroupProjection>,
    revision: &mut u64,
) {
    *items = Arc::new(Vec::new());
    *all_items = items.clone();
    *projection = Arc::new(Default::default());
    *revision = revision.wrapping_add(1);
}

impl ImageViewerApp {
    pub(crate) fn clear_listing_for_navigation(&mut self) {
        clear_listing(
            &mut self.items,
            &mut self.all_items,
            &mut self.group_projection,
            &mut self.items_revision,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_replaces_shared_storage_and_advances_revision() {
        let saved = Arc::new(vec![FileEntry {
            path: "file.txt".into(),
            name: "file.txt".into(),
            is_dir: false,
            size: 42,
            modified: 0,
            created: None,
            folder_cover: None,
            drive_info: None,
            sync_status: crate::domain::file_entry::SyncStatus::None,
            is_hidden: false,
            recycle_bin: None,
        }]);
        let mut items = saved.clone();
        let mut all_items = saved.clone();
        let mut projection = Arc::new(GroupProjection::default());
        let saved_projection = projection.clone();
        let mut revision = 7;

        clear_listing(&mut items, &mut all_items, &mut projection, &mut revision);

        assert!(items.is_empty());
        assert!(all_items.is_empty());
        assert!(Arc::ptr_eq(&items, &all_items));
        assert!(!Arc::ptr_eq(&saved, &all_items));
        assert_eq!(Arc::strong_count(&saved), 1);
        assert_eq!(saved[0].name, "file.txt");
        assert_eq!(saved[0].size, 42);
        assert!(!Arc::ptr_eq(&saved_projection, &projection));
        assert_eq!(revision, 8);
    }

    #[test]
    fn navigation_revision_wraps_like_other_listing_updates() {
        let mut items = Arc::new(Vec::new());
        let mut all_items = items.clone();
        let mut projection = Arc::new(GroupProjection::default());
        let mut revision = u64::MAX;

        clear_listing(&mut items, &mut all_items, &mut projection, &mut revision);

        assert_eq!(revision, 0);
    }
}
