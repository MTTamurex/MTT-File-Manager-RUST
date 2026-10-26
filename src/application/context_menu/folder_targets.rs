use super::ContextMenuState;
use std::collections::HashSet;
use std::path::Path;

impl ContextMenuState {
    /// Preserve menu order and classify only from already available metadata.
    pub(crate) fn capture_folder_targets<'a>(
        &mut self,
        folders: impl IntoIterator<Item = &'a Path>,
    ) {
        let folders: HashSet<_> = folders.into_iter().collect();
        self.folder_targets = self
            .target_paths
            .iter()
            .filter(|path| folders.contains(path.as_path()))
            .cloned()
            .collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui;

    #[test]
    fn captures_only_selected_folders_in_menu_order_and_clears_between_menus() {
        let mut menu = ContextMenuState::default();
        menu.open(
            egui::Pos2::ZERO,
            None,
            vec!["file.txt".into(), "Folder B".into(), "Folder A".into()],
            false,
        );
        menu.capture_folder_targets([
            Path::new("Folder A"),
            Path::new("Folder B"),
            Path::new("Other"),
        ]);
        assert_eq!(
            menu.folder_targets,
            vec![std::path::PathBuf::from("Folder B"), "Folder A".into()]
        );
        menu.close();
        assert!(menu.folder_targets.is_empty());
        menu.open(egui::Pos2::ZERO, None, vec!["file.txt".into()], false);
        assert!(menu.folder_targets.is_empty());
    }
}
