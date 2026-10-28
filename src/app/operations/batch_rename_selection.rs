//! Share the batch-rename source policy between the command bar and F2.
use crate::app::ImageViewerApp;
use crate::domain::file_entry::FileEntry;
use rustc_hash::FxHashSet;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

fn sources<'a>(
    items: Arc<Vec<FileEntry>>,
    selected: &'a FxHashSet<PathBuf>,
    mut allowed: impl FnMut(&Path) -> bool + 'a,
) -> impl Iterator<Item = PathBuf> + 'a {
    // Own the snapshot so Miller-column sources remain valid while iterating.
    (0..items.len()).filter_map(move |index| {
        let item = &items[index];
        (item.drive_info.is_none() && selected.contains(&item.path) && allowed(&item.path))
            .then(|| item.path.clone())
    })
}

impl ImageViewerApp {
    pub(crate) fn can_batch_rename(&self) -> bool {
        self.multi_selection.len() > 1
            && !self.navigation_state.is_recycle_bin_view
            && self.batch_rename_sources().take(2).count() == 2
    }

    pub(crate) fn batch_rename_sources(&self) -> impl Iterator<Item = PathBuf> + '_ {
        let current_path = Path::new(&self.navigation_state.current_path);
        let items = self
            .selected_file
            .as_ref()
            .and_then(|item| item.path.parent())
            .filter(|parent| *parent != current_path)
            .and_then(|parent| self.miller_columns.get_arc(parent))
            .unwrap_or_else(|| self.items.clone());
        sources(items, &self.multi_selection, |path| {
            !self.path_is_network_operation_root(path)
                && !crate::domain::file_entry::is_path_inside_existing_archive_file(path)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn items(names: &[&str]) -> Arc<Vec<FileEntry>> {
        Arc::new(
            names
                .iter()
                .map(|name| FileEntry::from_path(PathBuf::from(name), false))
                .collect(),
        )
    }

    #[test]
    fn multiple_eligible_files_enable_batch_rename_in_display_order() {
        let items = items(&["file2.txt", "file1.txt", "other.txt"]);
        let selected = [PathBuf::from("file1.txt"), PathBuf::from("file2.txt")]
            .into_iter()
            .collect();
        assert_eq!(
            sources(items.clone(), &selected, |_| true).take(2).count(),
            2
        );
        assert_eq!(
            sources(items, &selected, |_| true).collect::<Vec<_>>(),
            vec![PathBuf::from("file2.txt"), PathBuf::from("file1.txt")]
        );
    }

    #[test]
    fn blocked_and_unselected_items_do_not_enable_batch_rename() {
        let items = items(&["file1.txt", "file2.txt", "other.txt"]);
        let selected = [PathBuf::from("file1.txt"), PathBuf::from("file2.txt")]
            .into_iter()
            .collect();
        assert_eq!(
            sources(items, &selected, |path| path != Path::new("file2.txt"))
                .take(2)
                .count(),
            1
        );
    }

    #[test]
    fn availability_check_stops_after_two_eligible_files() {
        let items = items(&["file1.txt", "file2.txt", "file3.txt"]);
        let selected = items.iter().map(|item| item.path.clone()).collect();
        let calls = Cell::new(0);
        assert_eq!(
            sources(items, &selected, |_| {
                calls.set(calls.get() + 1);
                true
            })
            .take(2)
            .count(),
            2
        );
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn drives_are_never_counted_as_batch_rename_sources() {
        let mut items = items(&["drive", "file.txt"]);
        Arc::make_mut(&mut items)[0].drive_info = Some(crate::domain::file_entry::DriveInfo {
            file_system: "NTFS".into(),
            total_space: 0,
            free_space: 0,
            drive_type: crate::infrastructure::windows::system_info::DriveType::Fixed,
            model: None,
            serial_number: None,
            firmware_revision: None,
            bus_type: None,
            health: None,
        });
        let selected = items.iter().map(|item| item.path.clone()).collect();
        assert_eq!(
            sources(items, &selected, |_| true).collect::<Vec<_>>(),
            vec![PathBuf::from("file.txt")]
        );
    }

    #[test]
    fn empty_or_single_selection_never_provides_a_batch() {
        let items = items(&["file1.txt", "file2.txt"]);
        for selected in [
            FxHashSet::default(),
            [PathBuf::from("file1.txt")].into_iter().collect(),
        ] {
            assert!(sources(items.clone(), &selected, |_| true).take(2).count() < 2);
        }
    }

    #[test]
    fn stale_selection_paths_are_not_added_to_the_batch() {
        let items = items(&["file1.txt"]);
        let selected = [PathBuf::from("file1.txt"), PathBuf::from("missing.txt")]
            .into_iter()
            .collect();
        assert_eq!(
            sources(items, &selected, |_| true).collect::<Vec<_>>(),
            vec![PathBuf::from("file1.txt")]
        );
    }

    #[test]
    fn blocked_and_unselected_items_do_not_run_expensive_path_checks() {
        let mut items = items(&["drive", "other.txt", "file1.txt", "file2.txt"]);
        Arc::make_mut(&mut items)[0].drive_info = Some(crate::domain::file_entry::DriveInfo {
            file_system: "NTFS".into(),
            total_space: 0,
            free_space: 0,
            drive_type: crate::infrastructure::windows::system_info::DriveType::Fixed,
            model: None,
            serial_number: None,
            firmware_revision: None,
            bus_type: None,
            health: None,
        });
        let selected = [
            PathBuf::from("drive"),
            PathBuf::from("file1.txt"),
            PathBuf::from("file2.txt"),
        ]
        .into_iter()
        .collect();
        let checked = std::cell::RefCell::new(Vec::new());
        let count = sources(items, &selected, |path| {
            checked.borrow_mut().push(path.to_path_buf());
            true
        })
        .take(2)
        .count();
        assert_eq!(count, 2);
        assert_eq!(
            *checked.borrow(),
            vec![PathBuf::from("file1.txt"), PathBuf::from("file2.txt")]
        );
    }
}
