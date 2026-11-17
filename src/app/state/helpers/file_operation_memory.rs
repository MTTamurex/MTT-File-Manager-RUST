//! Cache-only memory policy while filesystem operations run.
use crate::ui::cache::{CacheManager, FxHashSet, MAX_DYNAMIC_TEXTURE_CACHE_ITEMS};
use crate::ui::icon_loader::IconLoader;
use std::path::PathBuf;

const CACHE_HEADROOM_ITEMS: usize = 16;
const RGBA_CACHE_BUDGET_BYTES: usize = 16 * 1024 * 1024;
const ICON_CACHE_ITEMS: usize = 128;

pub(super) fn cache_target(
    normal_target: usize,
    visible_items: usize,
    minimum_target: usize,
) -> usize {
    let floor = minimum_target.min(normal_target);
    normal_target.min(
        visible_items
            .saturating_add(CACHE_HEADROOM_ITEMS)
            .max(floor),
    )
}

pub(super) fn rgba_budget(normal_budget: usize) -> usize {
    normal_budget.min(RGBA_CACHE_BUDGET_BYTES)
}

pub(super) struct TrimmedCaches {
    pub textures_removed: usize,
    pub folder_previews_removed: usize,
    pub rgba_bytes_before: usize,
    pub icons_removed: usize,
    pub extension_icons_removed: usize,
}

pub(super) fn trim_active_caches(
    cache: &mut CacheManager,
    icons: &mut IconLoader,
    visible_paths: Option<&FxHashSet<PathBuf>>,
    texture_target: usize,
    folder_target: usize,
    rgba_budget: usize,
) -> Option<TrimmedCaches> {
    let rgba_bytes_before = cache.estimate_ram_cache_usage();
    let texture_capacity = cache.texture_cache.cap().get();
    let folder_capacity = cache.folder_preview_cache.cap().get();
    let rgba_capacity = cache.rgba_data_cache.cap().get();
    let rgba_item_target = texture_target
        .saturating_mul(3)
        .min(MAX_DYNAMIC_TEXTURE_CACHE_ITEMS);
    let (icon_items, extension_items, _, _, _) = icons.cache_counts();
    let needs_trim = cache.texture_cache.len() > texture_target
        || texture_capacity > texture_target
        || cache.folder_preview_cache.len() > folder_target
        || folder_capacity > folder_target
        || rgba_bytes_before > rgba_budget
        || rgba_capacity > rgba_item_target
        || icon_items > ICON_CACHE_ITEMS
        || extension_items > ICON_CACHE_ITEMS;
    if !needs_trim {
        return None;
    }

    let (textures_removed, _, folder_previews_removed) =
        cache.trim_thumbnail_caches(texture_target, rgba_budget, folder_target, visible_paths);
    // Trim texture and preview entries before shrinking capacities so their
    // eviction counts include all released LRU entries.
    cache.retune_texture_cache_capacity(texture_target);
    cache.retune_folder_preview_cache_capacity(folder_target);
    cache.retune_rgba_budget(rgba_budget);
    cache.retune_rgba_cache_capacity(rgba_item_target);
    let (icons_removed, extension_icons_removed) =
        icons.trim_icon_caches(ICON_CACHE_ITEMS, ICON_CACHE_ITEMS);

    Some(TrimmedCaches {
        textures_removed,
        folder_previews_removed,
        rgba_bytes_before,
        icons_removed,
        extension_icons_removed,
    })
}

#[cfg(test)]
mod tests {
    use super::{cache_target, rgba_budget, trim_active_caches};
    use crate::ui::cache::{CacheManager, FxHashSet};
    use crate::ui::icon_loader::IconLoader;
    use eframe::egui;
    use std::path::PathBuf;

    #[test]
    fn active_file_operations_cap_only_offscreen_cache_retention() {
        assert_eq!(cache_target(900, 100, 48), 116);
        assert_eq!(cache_target(48, 8, 48), 48);
        assert_eq!(cache_target(24, 0, 32), 24);
        assert_eq!(rgba_budget(64 * 1024 * 1024), 16 * 1024 * 1024);
        assert_eq!(rgba_budget(8 * 1024 * 1024), 8 * 1024 * 1024);
    }

    #[test]
    fn active_trim_keeps_visible_textures_and_does_not_touch_in_flight_markers() {
        let context = egui::Context::default();
        let mut cache = CacheManager::new();
        cache.retune_texture_cache_capacity(64);
        let in_flight = PathBuf::from("transfer-placeholder");
        assert!(cache.start_loading(in_flight.clone()));

        for index in 0..64 {
            let path = PathBuf::from(format!("cache-{index}.png"));
            let texture = context.load_texture(
                format!("cache-{index}"),
                egui::ColorImage::filled([1, 1], egui::Color32::WHITE),
                egui::TextureOptions::LINEAR,
            );
            cache.put_thumbnail(path, texture);
        }
        let visible = [PathBuf::from("cache-0.png"), PathBuf::from("cache-1.png")]
            .into_iter()
            .collect::<FxHashSet<_>>();
        let mut icons = IconLoader::new();

        let trim = trim_active_caches(&mut cache, &mut icons, Some(&visible), 48, 32, 4 << 20)
            .expect("oversized caches should be trimmed");

        assert_eq!(trim.textures_removed, 16);
        assert!(visible.iter().all(|path| cache.has_thumbnail(path)));
        assert_eq!(cache.texture_cache.cap().get(), 48);
        assert_eq!(cache.folder_preview_cache.cap().get(), 32);
        assert!(cache.is_loading(&in_flight));
    }
}
