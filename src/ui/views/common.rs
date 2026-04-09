//! Common helper functions for views
//! Follows .cursorrules: single responsibility, < 300 lines

use crate::domain::file_entry::{archive_type_label, FileEntry};
use rust_i18n::t;

/// Delay (in seconds) before showing a tooltip on hover.
pub const TOOLTIP_DELAY_SECS: f32 = 0.3;

/// Gets file type string for display
pub fn get_file_type_string(item: &FileEntry) -> String {
    if let Some(label) = archive_type_label(&item.name) {
        return label;
    }
    if item.is_dir {
        t!("file_types.folder").to_string()
    } else if let Some(ext) = item.path.extension() {
        let ext_str = ext.to_string_lossy().to_uppercase();
        if !ext_str.is_empty() {
            t!("file_info.file_generic", ext = ext_str).to_string()
        } else {
            t!("file_info.file_unknown").to_string()
        }
    } else {
        t!("file_info.file_unknown").to_string()
    }
}

/// Formats date for display
pub fn format_date(timestamp: u64) -> String {
    crate::infrastructure::windows::format_date(timestamp)
}

/// Formats size for display
pub fn format_size(size: u64) -> String {
    crate::infrastructure::windows::format_size(size)
}

#[derive(Clone, Copy)]
pub struct ViewportTracker {
    pub first_visible_index: usize,
    pub last_visible_index: usize,
    pub prefetch_rows: usize,
    pub columns: usize,
}

impl ViewportTracker {
    pub fn new() -> Self {
        Self {
            first_visible_index: 0,
            last_visible_index: 0,
            prefetch_rows: 2,
            columns: 1,
        }
    }

    pub fn get_prefetch_range(&self, total_items: usize) -> (usize, usize) {
        if total_items == 0 {
            return (0, 0);
        }
        let items_per_prefetch = self.prefetch_rows.saturating_mul(self.columns).max(1);
        let prefetch_start = self.first_visible_index.saturating_sub(items_per_prefetch);
        let last_visible = self.last_visible_index.min(total_items.saturating_sub(1));
        let prefetch_end = (last_visible + 1 + items_per_prefetch).min(total_items);
        (prefetch_start, prefetch_end)
    }

    pub fn is_visible(&self, index: usize) -> bool {
        index >= self.first_visible_index && index <= self.last_visible_index
    }
}

impl Default for ViewportTracker {
    fn default() -> Self {
        Self::new()
    }
}
