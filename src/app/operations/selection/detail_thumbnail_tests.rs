use super::best_effort_texture_available;
use crate::ui::cache::CacheManager;
use eframe::egui;
use std::num::NonZeroUsize;
use std::path::PathBuf;

fn texture(ctx: &egui::Context) -> egui::TextureHandle {
    ctx.load_texture(
        "thumbnail",
        egui::ColorImage::new([16, 16], vec![egui::Color32::WHITE; 16 * 16]),
        egui::TextureOptions::LINEAR,
    )
}

#[test]
fn evicted_media_preview_is_not_treated_as_an_available_best_effort_texture() {
    let ctx = egui::Context::default();
    for filename in ["image.png", "video.mp4"] {
        let mut cache = CacheManager::new();
        cache.texture_cache.resize(NonZeroUsize::new(1).unwrap());
        let path = PathBuf::from(filename);
        cache.note_attempted_thumbnail_bucket(&path, 512);
        cache.put_thumbnail(path.clone(), texture(&ctx));
        assert!(best_effort_texture_available(
            cache.texture_cache.peek(&path),
            cache.attempted_thumbnail_bucket_for(&path),
            512,
        ));

        // Scrolling loads another item and evicts the offscreen selection.
        cache.put_thumbnail(PathBuf::from("other.png"), texture(&ctx));
        assert!(cache.texture_cache.peek(&path).is_none());
        assert_eq!(cache.attempted_thumbnail_bucket_for(&path), Some(512));
        assert!(!best_effort_texture_available(
            cache.texture_cache.peek(&path),
            cache.attempted_thumbnail_bucket_for(&path),
            512,
        ));

        // Reloading the selected item restores the no-repeat protection.
        cache.put_thumbnail(path.clone(), texture(&ctx));
        assert!(best_effort_texture_available(
            cache.texture_cache.peek(&path),
            cache.attempted_thumbnail_bucket_for(&path),
            512,
        ));
    }
}

#[test]
fn cached_small_texture_is_accepted_only_after_attempting_required_quality() {
    let ctx = egui::Context::default();
    let tex = texture(&ctx);
    assert!(!best_effort_texture_available(Some(&tex), None, 512));
    assert!(!best_effort_texture_available(Some(&tex), Some(128), 512));
    assert!(best_effort_texture_available(Some(&tex), Some(512), 512));
    assert!(best_effort_texture_available(Some(&tex), Some(1024), 512));
    assert!(!best_effort_texture_available(None, Some(512), 512));
}
