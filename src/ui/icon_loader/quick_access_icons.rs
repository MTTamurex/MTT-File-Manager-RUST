use super::*;
use std::path::Path;

fn uses_native_icon(path: &str, registered: bool) -> bool {
    let drive = path
        .strip_prefix(r"\\?\")
        .unwrap_or(path)
        .trim_end_matches(['\\', '/'])
        .as_bytes();
    registered
        || (drive.len() == 2 && drive[0].is_ascii_alphabetic() && drive[1] == b':')
        || crate::domain::network_path::is_network_location_root(Path::new(path))
        || crate::infrastructure::onedrive::is_special_icon_folder(Path::new(path))
}

impl IconLoader {
    /// Quick Access shares the selected generic artwork, not cached native yellow folders.
    pub fn get_or_load_quick_access_folder_icon(
        &mut self,
        ctx: &egui::Context,
        path: &str,
    ) -> Option<egui::TextureHandle> {
        if uses_native_icon(path, self.has_registered_folder_icon(path)) {
            self.get_or_load_registered_folder_icon(ctx, path)
                .or_else(|| self.get_or_load_folder_path_icon(ctx, path))
        } else {
            self.folder_icon()
                .cloned()
                .or_else(|| self.get_or_load_folder_path_icon(ctx, path))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quick_access_regular_folders_follow_replaced_folder_artwork_immediately() {
        let ctx = egui::Context::default();
        let mut loader = IconLoader::new();
        for color in [
            [232, 216, 138, 255],
            [70, 93, 112, 255],
            [255, 210, 60, 255],
        ] {
            loader.set_folder_icon(&ctx, &color, 1, 1);
            let selected = loader.folder_icon().unwrap().id();
            let quick_access = loader
                .get_or_load_quick_access_folder_icon(&ctx, r"C:\Library\Folder")
                .unwrap();
            assert_eq!(quick_access.id(), selected);
        }
    }

    #[test]
    fn quick_access_preserves_native_drive_network_and_registered_icons() {
        assert!(uses_native_icon("C:\\", false));
        assert!(uses_native_icon(r"\\?\C:\", false));
        assert!(uses_native_icon(r"\\server\share", false));
        assert!(uses_native_icon(r"C:\Library\Folder", true));
        assert!(!uses_native_icon(r"C:\Library\Folder", false));
    }
}
