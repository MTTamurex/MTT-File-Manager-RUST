//! Completed metadata snapshots are shared by tab clones; pending work is owned by the active view.
use super::*;
use crate::{application::grouping::columns::ColumnGroups, domain::list_columns::ColumnId};

#[derive(Clone)]
pub(super) struct CachedMetadata {
    signature: u64,
    items: Arc<Vec<FileEntry>>,
    descending: bool,
    groups: Arc<ColumnGroups>,
    projection: Arc<GroupProjection>,
}

#[derive(Clone)]
pub(super) struct CachedSource {
    items: Arc<Vec<FileEntry>>,
    fingerprint: u64,
}

impl ColumnGroupingCache {
    pub fn metadata_fingerprint(
        &mut self,
        directory: &Path,
        items: &Arc<Vec<FileEntry>>,
        compute: impl FnOnce(&Arc<Vec<FileEntry>>) -> u64,
    ) -> u64 {
        if let Some(cached) = self.2.get(directory) {
            if Arc::ptr_eq(&cached.items, items) {
                return cached.fingerprint;
            }
        }
        let fingerprint = compute(items);
        self.2.insert(
            directory.to_owned(),
            CachedSource {
                items: items.clone(),
                fingerprint,
            },
        );
        fingerprint
    }

    pub fn metadata_groups(&self, directory: &Path, signature: u64) -> Option<Arc<ColumnGroups>> {
        self.1
            .get(directory)
            .filter(|cached| cached.signature == signature)
            .map(|cached| cached.groups.clone())
    }

    pub fn metadata_projection(
        &mut self,
        directory: &Path,
        signature: u64,
        items: Arc<Vec<FileEntry>>,
        column: ColumnId,
        descending: bool,
        groups: Option<Arc<ColumnGroups>>,
    ) -> Arc<GroupProjection> {
        let cached = self.1.get(directory).filter(|cached| {
            cached.signature == signature
                && Arc::ptr_eq(&cached.items, &items)
                && cached.descending == descending
        });
        if let Some(cached) = cached {
            if groups
                .as_ref()
                .is_none_or(|groups| Arc::ptr_eq(groups, &cached.groups))
            {
                return cached.projection.clone();
            }
        }
        let Some(groups) = groups else {
            return Arc::default();
        };
        let projection = Arc::new(groups.project(&items, column, descending));
        self.1.insert(
            directory.to_owned(),
            CachedMetadata {
                signature,
                items,
                descending,
                groups,
                projection: projection.clone(),
            },
        );
        projection
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fingerprints_are_reused_across_long_ancestor_chains_and_refreshed_per_listing() {
        let mut cache = ColumnGroupingCache::default();
        let listings: Vec<_> = (0..12)
            .map(|index| {
                (
                    PathBuf::from(format!("folder_{index}")),
                    Arc::new(Vec::new()),
                )
            })
            .collect();
        for (index, (directory, items)) in listings.iter().enumerate() {
            assert_eq!(
                cache.metadata_fingerprint(directory, items, |_| index as u64),
                index as u64
            );
        }
        let mut cloned = cache.clone();
        for (index, (directory, items)) in listings.iter().enumerate() {
            assert_eq!(
                cloned.metadata_fingerprint(directory, items, |_| panic!(
                    "unchanged listing must reuse its fingerprint"
                )),
                index as u64
            );
        }
        let (directory, _) = &listings[0];
        assert_eq!(
            cloned.metadata_fingerprint(directory, &Arc::new(Vec::new()), |_| 99),
            99
        );
        cloned.invalidate(directory);
        assert_eq!(
            cloned.metadata_fingerprint(directory, &listings[0].1, |_| 100),
            100
        );
    }

    #[test]
    fn metadata_groups_survive_tab_clones_reverse_without_reloading_and_invalidate_by_source() {
        let mut cache = ColumnGroupingCache::default();
        let directory = Path::new(r"C:\Example");
        let items = Arc::new(vec![FileEntry::from_path("file.txt".into(), false)]);
        let groups = Arc::new(ColumnGroups::default());
        let first = cache.metadata_projection(
            directory,
            1,
            items.clone(),
            ColumnId::Tags,
            false,
            Some(groups.clone()),
        );
        let mut cloned = cache.clone();
        assert!(Arc::ptr_eq(
            &first,
            &cloned.metadata_projection(directory, 1, items.clone(), ColumnId::Tags, false, None)
        ));
        let reversed = cloned.metadata_projection(
            directory,
            1,
            items.clone(),
            ColumnId::Tags,
            true,
            Some(groups.clone()),
        );
        assert!(!Arc::ptr_eq(&first, &reversed));
        assert!(Arc::ptr_eq(
            &groups,
            &cloned.metadata_groups(directory, 1).unwrap()
        ));
        assert!(cloned.metadata_groups(directory, 2).is_none());
        cloned.invalidate(directory);
        assert!(cloned.metadata_groups(directory, 1).is_none());
    }
}
