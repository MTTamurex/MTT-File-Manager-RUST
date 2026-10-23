use super::*;

#[test]
fn font_changes_clear_both_width_and_truncation_caches() {
    FONT_WIDTH_CACHE.with(|cache| cache.borrow_mut().put(1, 42.0));
    TRUNCATION_CACHE.with(|cache| cache.borrow_mut().put(1, "Sample…".into()));
    clear_font_metrics_cache();
    FONT_WIDTH_CACHE.with(|cache| assert!(cache.borrow().is_empty()));
    TRUNCATION_CACHE.with(|cache| assert!(cache.borrow().is_empty()));
}
