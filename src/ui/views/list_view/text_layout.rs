//! Bounded caches for list-view text measurement and truncation.
use eframe::egui::{Color32, FontId, Ui};
use lru::LruCache;
use std::sync::atomic::{AtomicU64, Ordering};
use std::{
    cell::RefCell,
    hash::{Hash, Hasher},
    num::NonZeroUsize,
};

static FONT_METRICS_REVISION: AtomicU64 = AtomicU64::new(0);

pub(super) fn font_metrics_revision() -> u64 {
    FONT_METRICS_REVISION.load(Ordering::Relaxed)
}

thread_local! {
    static FONT_WIDTH_CACHE: RefCell<LruCache<u64, f32>> = RefCell::new(LruCache::new(NonZeroUsize::new(5000).unwrap()));
    static TRUNCATION_CACHE: RefCell<LruCache<u64, String>> = RefCell::new(LruCache::new(NonZeroUsize::new(2000).unwrap()));
}

pub(super) fn clear_truncation_cache() {
    TRUNCATION_CACHE.with(|cache| cache.borrow_mut().clear());
}

pub(crate) fn clear_font_metrics_cache() {
    FONT_METRICS_REVISION.fetch_add(1, Ordering::Relaxed);
    FONT_WIDTH_CACHE.with(|cache| cache.borrow_mut().clear());
    clear_truncation_cache();
}

pub(super) fn get_cached_text_width(text: &str, font: &FontId, color: Color32, ui: &Ui) -> f32 {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hash);
    font.size.to_bits().hash(&mut hash);
    let key = hash.finish();
    FONT_WIDTH_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(width) = cache.get(&key) {
            return *width;
        }
        let width = ui
            .painter()
            .layout_no_wrap(text.to_owned(), font.clone(), color)
            .rect
            .width();
        cache.put(key, width);
        width
    })
}

pub(crate) fn truncate_text_for_column(
    text: &str,
    max_width: f32,
    font: &FontId,
    ui: &Ui,
) -> String {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hash);
    max_width.to_bits().hash(&mut hash);
    font.size.to_bits().hash(&mut hash);
    let key = hash.finish();
    if let Some(value) = TRUNCATION_CACHE.with(|cache| cache.borrow_mut().get(&key).cloned()) {
        return value;
    }
    let result = if get_cached_text_width(text, font, Color32::WHITE, ui) <= max_width {
        text.to_owned()
    } else {
        let available = max_width - get_cached_text_width("...", font, Color32::WHITE, ui);
        let boundaries: Vec<_> = text
            .char_indices()
            .map(|(index, _)| index)
            .chain(std::iter::once(text.len()))
            .collect();
        let (mut left, mut right) = (0, boundaries.len() - 1);
        while left < right {
            let middle = (left + right).div_ceil(2);
            if get_cached_text_width(&text[..boundaries[middle]], font, Color32::WHITE, ui)
                <= available
            {
                left = middle;
            } else {
                right = middle - 1;
            }
        }
        format!("{}...", &text[..boundaries[left]])
    };
    TRUNCATION_CACHE.with(|cache| cache.borrow_mut().put(key, result.clone()));
    result
}

#[cfg(test)]
#[path = "font_metrics_tests.rs"]
mod tests;
