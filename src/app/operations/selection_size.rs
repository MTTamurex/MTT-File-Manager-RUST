//! Resolve the listing that owns the selection without filesystem I/O.
use crate::app::folder_size_state::SelectionSizeSummary;
use crate::app::state::ImageViewerApp;
use crate::domain::file_entry::ViewMode;
use crate::ui::cache::FxHashSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

impl ImageViewerApp {
    pub(crate) fn selected_size_summary(&mut self) -> SelectionSizeSummary {
        let ancestor_listing = ancestor_selection_directory(
            self.view_mode,
            Path::new(&self.navigation_state.current_path),
            &self.multi_selection,
        )
        .and_then(|parent| self.miller_columns.get_arc(parent));
        // A filter can hide selected entries. Their sizes must still count.
        // During a reload, however, all_items can be only a partial new listing
        // while the user is still interacting with the preserved visible one.
        let current_listing = if self.hold_visible_items_until_load_complete {
            &self.items
        } else {
            &self.all_items
        };
        let items = ancestor_listing.as_ref().unwrap_or(current_listing);
        self.folder_size_state
            .selection_size(items, &self.multi_selection, Instant::now())
    }
}

fn ancestor_selection_directory<'a>(
    mode: ViewMode,
    current_path: &Path,
    selected: &'a FxHashSet<PathBuf>,
) -> Option<&'a Path> {
    if mode != ViewMode::Miller {
        return None;
    }
    let parent = selected.iter().next()?.parent()?;
    (parent != current_path && selected.iter().all(|path| path.parent() == Some(parent)))
        .then_some(parent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn miller_ancestor_selection_uses_its_own_listing() {
        let selected = FxHashSet::from_iter([
            PathBuf::from("parent/folder-a"),
            PathBuf::from("parent/folder-b"),
        ]);
        assert_eq!(
            ancestor_selection_directory(ViewMode::Miller, Path::new("parent/current"), &selected),
            Some(Path::new("parent"))
        );
        assert_eq!(
            ancestor_selection_directory(ViewMode::Miller, Path::new("parent"), &selected),
            None
        );
        assert_eq!(
            ancestor_selection_directory(ViewMode::Grid, Path::new("parent/current"), &selected),
            None
        );
    }

    #[test]
    fn mixed_or_empty_selection_does_not_choose_an_arbitrary_ancestor() {
        let selected = FxHashSet::from_iter([
            PathBuf::from("parent/folder-a"),
            PathBuf::from("other/folder-b"),
        ]);
        assert_eq!(
            ancestor_selection_directory(ViewMode::Miller, Path::new("current"), &selected),
            None
        );
        assert_eq!(
            ancestor_selection_directory(
                ViewMode::Miller,
                Path::new("current"),
                &FxHashSet::default()
            ),
            None
        );
    }
}
