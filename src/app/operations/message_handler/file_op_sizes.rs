use crate::app::state::ImageViewerApp;
use crate::ui::cache::FxHashSet;
use std::path::PathBuf;

// The direct parents are invalidated with their listings by the event handler.
// Recursive sizes also change above them, even though those listings did not change.
fn changed_content_ancestors(parent_folders: &[PathBuf]) -> FxHashSet<PathBuf> {
    let mut ancestors = FxHashSet::default();
    for parent in parent_folders {
        for ancestor in parent.ancestors().skip(1) {
            if ancestor.as_os_str().is_empty() {
                break;
            }
            if !ancestors.insert(ancestor.to_path_buf()) {
                break;
            }
        }
    }
    ancestors
}

impl ImageViewerApp {
    pub(super) fn invalidate_content_ancestor_sizes(&mut self, parents: &[PathBuf]) {
        for ancestor in changed_content_ancestors(parents) {
            // Use the normal invalidation path to reject in-flight stale results
            // and revalidate values fetched before the search index catches up.
            self.invalidate_folder_size_cache(&ancestor);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_deletion_invalidates_sizes_up_to_drive_root() {
        let ancestors = changed_content_ancestors(&[PathBuf::from(r"C:\Data\Folder\Nested")]);

        assert_eq!(
            ancestors,
            FxHashSet::from_iter([
                PathBuf::from(r"C:\Data\Folder"),
                PathBuf::from(r"C:\Data"),
                PathBuf::from(r"C:\"),
            ])
        );
    }

    #[test]
    fn batch_deletion_covers_shared_and_separate_ancestor_chains() {
        let ancestors = changed_content_ancestors(&[
            PathBuf::from(r"C:\Data\First"),
            PathBuf::from(r"C:\Data\Second"),
            PathBuf::from(r"D:\Other\Folder"),
        ]);

        assert_eq!(
            ancestors,
            FxHashSet::from_iter([
                PathBuf::from(r"C:\Data"),
                PathBuf::from(r"C:\"),
                PathBuf::from(r"D:\Other"),
                PathBuf::from(r"D:\"),
            ])
        );
    }

    #[test]
    fn deletion_at_drive_root_has_no_additional_ancestors() {
        assert!(changed_content_ancestors(&[PathBuf::from(r"C:\")]).is_empty());
    }

    #[test]
    fn watcher_content_changes_cover_nested_and_overlapping_parents() {
        let ancestors = changed_content_ancestors(&[
            PathBuf::from(r"C:\Data\Folder\Nested"),
            PathBuf::from(r"C:\Data\Folder"),
        ]);
        assert_eq!(
            ancestors,
            FxHashSet::from_iter([
                PathBuf::from(r"C:\Data\Folder"),
                PathBuf::from(r"C:\Data"),
                PathBuf::from(r"C:\"),
            ])
        );
    }
}
