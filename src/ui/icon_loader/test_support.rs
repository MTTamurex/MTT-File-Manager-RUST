//! Cached icons for interaction tests that must not load native Shell providers.
use super::*;

impl IconLoader {
    pub(crate) fn with_cached_test_icons(ctx: &egui::Context, paths: &[&str]) -> Self {
        let mut loader = Self::new();
        let icon = ctx.load_texture(
            "test_sidebar_icon",
            egui::ColorImage::filled([1, 1], egui::Color32::WHITE),
            egui::TextureOptions::LINEAR,
        );
        loader.folder_icon_texture = Some(icon.clone());
        loader.computer_icon_texture = Some(icon.clone());
        loader.recycle_bin_icon_texture = Some(icon.clone());
        for path in paths {
            for key in [
                (*path).to_owned(),
                async_ops::folder_path_icon_cache_key(path),
                async_ops::cloud_root_icon_cache_key(path, None),
            ] {
                loader.drive_icon_cache.put(key, icon.clone());
            }
        }
        loader.drive_icon_cache.put(
            network_location_icons::NETWORK_LOCATION_ICON_KEY.into(),
            icon,
        );
        loader
    }
}

#[test]
fn interaction_fixture_resolves_icons_without_native_workers() {
    let ctx = egui::Context::default();
    let mut loader = IconLoader::with_cached_test_icons(&ctx, &[r"C:\", r"C:\Cloud"]);
    assert!(loader.get_or_load_drive_icon(&ctx, r"C:\").is_some());
    assert!(loader
        .get_or_load_cloud_root_icon(&ctx, r"C:\Cloud", None)
        .is_some());
    assert!(loader
        .get_or_load_folder_path_icon(&ctx, r"C:\Cloud")
        .is_some());
    assert!(loader.get_or_load_network_location_icon(&ctx).is_some());
    assert_eq!(loader.auxiliary_icon_thread_count(), 0);
    assert!(!loader.has_pending_auxiliary_icon_work());
}
