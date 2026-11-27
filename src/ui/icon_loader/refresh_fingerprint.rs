//! Keeps refreshed icons whose pixels did not change.
//!
//! Drive and cloud-root icons are re-extracted on every drive scan so provider
//! icon changes show up. Uploading an identical image as a new texture changes
//! the frame anyway (new texture id), which makes an idle window render and
//! present on the GPU. A fingerprint of the uploaded pixels lets an unchanged
//! refresh keep the texture already on screen.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use eframe::egui;

#[derive(Default)]
pub(super) struct IconFingerprints {
    /// Pixel fingerprint and the texture it produced, per cache key.
    by_key: HashMap<String, (u64, egui::TextureId)>,
}

pub(super) fn fingerprint(width: u32, height: u32, rgba: &[u8]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    width.hash(&mut hasher);
    height.hash(&mut hasher);
    rgba.hash(&mut hasher);
    hasher.finish()
}

impl IconFingerprints {
    /// Whether `texture` (currently cached for `key`) already shows these pixels.
    pub(super) fn unchanged(
        &self,
        key: &str,
        texture: Option<&egui::TextureHandle>,
        fingerprint: u64,
    ) -> bool {
        texture.is_some_and(|texture| self.by_key.get(key) == Some(&(fingerprint, texture.id())))
    }

    pub(super) fn record(&mut self, key: String, fingerprint: u64, texture: &egui::TextureHandle) {
        self.by_key.insert(key, (fingerprint, texture.id()));
    }

    pub(super) fn clear(&mut self) {
        self.by_key.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texture(ctx: &egui::Context, name: &str) -> egui::TextureHandle {
        ctx.load_texture(
            name,
            egui::ColorImage::new([1, 1], vec![egui::Color32::WHITE]),
            egui::TextureOptions::LINEAR,
        )
    }

    #[test]
    fn identical_pixels_for_the_same_texture_are_unchanged() {
        let ctx = egui::Context::default();
        let shown = texture(&ctx, "icon");
        let mut prints = IconFingerprints::default();
        let pixels = fingerprint(1, 1, &[1, 2, 3, 4]);
        prints.record("key".into(), pixels, &shown);

        assert!(prints.unchanged("key", Some(&shown), pixels));
        assert!(!prints.unchanged("key", Some(&shown), fingerprint(1, 1, &[9, 2, 3, 4])));
        assert!(!prints.unchanged("key", None, pixels));
        assert!(!prints.unchanged("other", Some(&shown), pixels));
    }

    #[test]
    fn a_texture_replaced_by_another_path_is_not_trusted() {
        let ctx = egui::Context::default();
        let first = texture(&ctx, "first");
        let replaced = texture(&ctx, "replaced");
        let mut prints = IconFingerprints::default();
        let pixels = fingerprint(1, 1, &[1, 2, 3, 4]);
        prints.record("key".into(), pixels, &first);

        assert!(!prints.unchanged("key", Some(&replaced), pixels));
    }
}
