//! Cached projections; changing grouping must not re-enumerate directories.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::application::grouping::{build_group_projection, local_grouping_day, GroupProjection};
use crate::domain::file_entry::{FileEntry, GroupMode};
mod metadata;

#[derive(Clone)]
struct CachedProjection {
    items: Arc<Vec<FileEntry>>,
    mode: GroupMode,
    descending: bool,
    day: Option<i64>,
    projection: Arc<GroupProjection>,
}

#[derive(Clone, Default)]
pub(crate) struct ColumnGroupingCache(
    HashMap<PathBuf, CachedProjection>,
    HashMap<PathBuf, metadata::CachedMetadata>,
    HashMap<PathBuf, metadata::CachedSource>,
);

impl ColumnGroupingCache {
    pub fn projection(
        &mut self,
        directory: &Path,
        items: Arc<Vec<FileEntry>>,
        mode: GroupMode,
        descending: bool,
    ) -> Arc<GroupProjection> {
        let day = if mode == GroupMode::Date {
            local_grouping_day()
        } else {
            None
        };
        if let Some(cached) = self.0.get(directory) {
            if Arc::ptr_eq(&cached.items, &items)
                && cached.mode == mode
                && cached.descending == descending
                && cached.day == day
            {
                return cached.projection.clone();
            }
        }
        let projection = Arc::new(build_group_projection(&items, mode, descending));
        self.0.insert(
            directory.to_path_buf(),
            CachedProjection {
                items,
                mode,
                descending,
                day,
                projection: projection.clone(),
            },
        );
        projection
    }

    pub fn retain(&mut self, keep: &std::collections::HashSet<PathBuf>) {
        self.0.retain(|path, _| keep.contains(path));
        self.1.retain(|path, _| keep.contains(path));
        self.2.retain(|path, _| keep.contains(path));
    }

    pub fn invalidate(&mut self, directory: &Path) {
        self.0.remove(directory);
        self.1.remove(directory);
        self.2.remove(directory);
    }

    pub fn clear(&mut self) {
        self.0.clear();
        self.1.clear();
        self.2.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(names: &[&str]) -> Arc<Vec<FileEntry>> {
        Arc::new(
            names
                .iter()
                .map(|name| FileEntry {
                    path: PathBuf::from(name),
                    name: name.to_string(),
                    is_dir: false,
                    size: 1,
                    modified: 0,
                    created: None,
                    folder_cover: None,
                    drive_info: None,
                    sync_status: Default::default(),
                    is_hidden: false,
                    recycle_bin: None,
                })
                .collect(),
        )
    }

    #[test]
    fn projections_follow_listing_mode_and_direction_without_reloading() {
        let directory = Path::new("folder");
        let listing = items(&["alpha.txt", "zeta.txt"]);
        let mut cache = ColumnGroupingCache::default();
        let original = cache.projection(directory, listing.clone(), GroupMode::Name, false);
        let reused = cache.projection(directory, listing.clone(), GroupMode::Name, false);
        assert!(Arc::ptr_eq(&original, &reused));
        let reversed = cache.projection(directory, listing.clone(), GroupMode::Name, true);
        assert_eq!(
            original.sections.first().unwrap().key,
            reversed.sections.last().unwrap().key
        );
        assert!(!Arc::ptr_eq(&original, &reversed));
        for mode in [GroupMode::Date, GroupMode::Type, GroupMode::Size] {
            let projection = cache.projection(directory, listing.clone(), mode, false);
            assert_eq!(*projection, build_group_projection(&listing, mode, false));
        }
        let changed = cache.projection(directory, items(&["file.txt"]), GroupMode::Name, false);
        assert_eq!(changed.item_count(), 1);
        let disabled = cache.projection(directory, listing, GroupMode::None, false);
        assert!(!disabled.is_grouped());
    }

    #[test]
    fn columns_have_independent_projections_and_invalidation() {
        let mut cache = ColumnGroupingCache::default();
        let first = Path::new("parent");
        let second = Path::new("parent/child");
        let listing = items(&["file.txt"]);
        let parent = cache.projection(first, listing.clone(), GroupMode::Name, false);
        let child = cache.projection(second, listing.clone(), GroupMode::Name, false);
        cache.invalidate(first);
        assert!(!Arc::ptr_eq(
            &parent,
            &cache.projection(first, listing.clone(), GroupMode::Name, false)
        ));
        assert!(Arc::ptr_eq(
            &child,
            &cache.projection(second, listing, GroupMode::Name, false)
        ));
        cache.retain(&std::collections::HashSet::from([first.to_path_buf()]));
        assert_eq!(cache.0.len(), 1);
        cache.clear();
        assert!(cache.0.is_empty());
    }
}
