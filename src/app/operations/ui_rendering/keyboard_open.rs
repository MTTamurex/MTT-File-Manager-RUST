use crate::app::ImageViewerApp;
use crate::domain::file_entry::FileEntry;
use crate::ui::cache::FxHashSet;
use eframe::egui;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

fn selected_folder_paths(
    listing: &[FileEntry],
    selection: &FxHashSet<PathBuf>,
    focused: Option<&Path>,
) -> Vec<String> {
    let Some(primary) = focused
        .filter(|path| selection.contains(*path))
        .or_else(|| selection.iter().next().map(PathBuf::as_path))
    else {
        return Vec::new();
    };
    let folders: HashSet<_> = listing
        .iter()
        .filter(|item| (item.is_dir || item.drive_info.is_some()) && !item.is_recycle_item())
        .map(|item| item.path.as_path())
        .collect();
    super::ordered_context_menu_paths(primary, selection)
        .into_iter()
        .filter(|path| folders.contains(path.as_path()))
        .map(|path| path.to_string_lossy().into_owned())
        .collect()
}

impl ImageViewerApp {
    /// Use the same batch operation as Open, but only after both panes render.
    /// Returns true when Enter is handled, including selections with no folders.
    pub(super) fn queue_selected_folders_open(&self, ctx: &egui::Context) -> bool {
        let ctrl = ctx.input(|input| input.modifiers.ctrl);
        if self.multi_selection.len() <= 1 {
            if ctrl && !self.navigation_state.is_recycle_bin_view {
                if let Some(item) = self
                    .selected_file
                    .as_ref()
                    .filter(|item| item.is_dir && !item.is_recycle_item())
                {
                    crate::ui::new_tab_request::queue(ctx, &item.path.to_string_lossy(), ctrl);
                    return true;
                }
            }
            return false;
        }
        if self.navigation_state.is_recycle_bin_view {
            return true;
        }
        // Ancestor-column selection is not stored in the focused column's items.
        let ancestor_listing = self
            .selected_file
            .as_ref()
            .filter(|_| self.selected_item.is_none())
            .and_then(|item| item.path.parent())
            .and_then(|directory| self.miller_columns.get_arc(directory));
        let listing = ancestor_listing.as_deref().unwrap_or(&self.items);
        let paths = selected_folder_paths(
            listing,
            &self.multi_selection,
            self.selected_file.as_ref().map(|item| item.path.as_path()),
        );
        crate::ui::new_tab_request::queue_paths(ctx, paths, ctrl);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enter_opens_every_selected_folder_in_the_same_order_as_the_context_menu() {
        let listing: Vec<_> = ["Folder A", "Folder B", "Folder C"]
            .into_iter()
            .map(|path| FileEntry::from_path(path.into(), true))
            .collect();
        let selection: FxHashSet<_> = listing.iter().map(|item| item.path.clone()).collect();
        let primary = Path::new("Folder C");
        let expected: Vec<_> = super::super::ordered_context_menu_paths(primary, &selection)
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect();
        let paths = selected_folder_paths(&listing, &selection, Some(primary));
        assert_eq!(paths, expected);
        assert_eq!(paths.len(), 3);
    }

    #[test]
    fn focused_file_does_not_prevent_opening_selected_folders() {
        let listing = [
            FileEntry::from_path("Folder A".into(), true),
            FileEntry::from_path("file.txt".into(), false),
            FileEntry::from_path("Folder B".into(), true),
        ];
        let selection = listing.iter().map(|item| item.path.clone()).collect();
        let paths = selected_folder_paths(&listing, &selection, Some(Path::new("file.txt")));
        assert_eq!(paths.len(), 2);
        assert!(paths.contains(&"Folder A".to_owned()));
        assert!(paths.contains(&"Folder B".to_owned()));
    }

    #[test]
    fn unselected_folders_and_file_only_selections_are_not_opened() {
        let listing = [
            FileEntry::from_path("Folder".into(), true),
            FileEntry::from_path("file.txt".into(), false),
        ];
        let selection = [PathBuf::from("file.txt"), PathBuf::from("other.txt")]
            .into_iter()
            .collect();
        assert!(selected_folder_paths(&listing, &selection, Some(Path::new("Folder"))).is_empty());
    }
}
