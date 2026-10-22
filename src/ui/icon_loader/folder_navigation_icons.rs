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
    /// Quick Access and tabs share the selected artwork, not cached native yellow folders.
    pub fn get_or_load_folder_navigation_icon(
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
    use crate::domain::folder_appearance::FolderAppearance;
    use crate::infrastructure::folder_compose::FolderComposer;

    #[test]
    fn regular_folders_follow_all_appearances_despite_cached_native_icons() {
        let ctx = egui::Context::default();
        let mut loader = IconLoader::new();
        let path = r"C:\Library\Folder";
        let native = ctx.load_texture(
            "native_folder",
            egui::ColorImage::filled([1, 1], egui::Color32::YELLOW),
            egui::TextureOptions::LINEAR,
        );
        loader
            .drive_icon_cache
            .put(path.to_lowercase(), native.clone());
        for appearance in FolderAppearance::ALL
            .into_iter()
            .chain([FolderAppearance::Original])
        {
            let (pixels, width, height) = FolderComposer::try_new()
                .unwrap()
                .with_appearance(appearance)
                .compose_empty();
            loader.set_folder_icon(&ctx, &pixels, width, height);
            let selected = loader.folder_icon().unwrap().id();
            let icon = loader
                .get_or_load_folder_navigation_icon(&ctx, path)
                .unwrap();
            assert_eq!(icon.id(), selected, "{appearance:?}");
            assert_ne!(icon.id(), native.id());
        }
    }

    #[test]
    fn navigation_preserves_native_drive_network_and_registered_icons() {
        assert!(uses_native_icon("C:\\", false));
        assert!(uses_native_icon(r"\\?\C:\", false));
        assert!(uses_native_icon(r"\\server\share", false));
        assert!(uses_native_icon(r"C:\Library\Folder", true));
        assert!(!uses_native_icon(r"C:\Library\Folder", false));
    }

    #[test]
    fn special_locations_use_cached_native_icons_instead_of_selected_artwork() {
        let ctx = egui::Context::default();
        let mut loader = IconLoader::new();
        loader.set_folder_icon(&ctx, &[70, 93, 112, 255], 1, 1);
        for path in ["C:\\", r"\\server\share", r"C:\Library\Provider"] {
            let key = path.to_lowercase().trim_end_matches('\\').to_string();
            if path == r"C:\Library\Provider" {
                loader
                    .registered_folder_icon_resources
                    .insert(key.clone(), "provider.ico".into());
            }
            let native = ctx.load_texture(
                "special_location",
                egui::ColorImage::filled([1, 1], egui::Color32::WHITE),
                egui::TextureOptions::LINEAR,
            );
            if path == r"C:\Library\Provider" {
                let cloud_key =
                    super::super::async_ops::cloud_root_icon_cache_key(path, Some("provider.ico"));
                loader.drive_icon_cache.put(cloud_key, native.clone());
            } else {
                loader.drive_icon_cache.put(key, native.clone());
            }
            let icon = loader
                .get_or_load_folder_navigation_icon(&ctx, path)
                .unwrap();
            assert_eq!(icon.id(), native.id());
            assert_ne!(icon.id(), loader.folder_icon().unwrap().id());
        }
    }
}
