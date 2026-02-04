//! Common helper functions for views
//! Follows .cursorrules: single responsibility, < 300 lines

use crate::domain::file_entry::FileEntry;
use crate::infrastructure::windows;

/// Gets file type string for display
pub fn get_file_type_string(item: &FileEntry) -> String {
    // Check for ZIP manually because is_dir might be true
    if item.is_zip() {
        return "Arquivo ZIP".to_string();
    }
    if item.is_dir {
        "Pasta".to_string()
    } else if let Some(ext) = item.path.extension() {
        let ext_str = ext.to_string_lossy().to_uppercase();
        if !ext_str.is_empty() {
            format!("Arquivo {}", ext_str)
        } else {
            "Arquivo".to_string()
        }
    } else {
        "Arquivo".to_string()
    }
}

/// Formats date for display
pub fn format_date(timestamp: u64) -> String {
    windows::format_date(timestamp)
}

/// Formats size for display
pub fn format_size(size: u64) -> String {
    windows::format_size(size)
}

/// Opens file with shell
pub fn open_with_shell(path: &std::path::Path) {
    windows::open_with_shell(path);
}

#[derive(Clone, Copy)]
pub struct ViewportTracker {
    pub first_visible_index: usize,
    pub last_visible_index: usize,
    pub prefetch_rows: usize,
    pub columns: usize,
}

impl ViewportTracker {
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

#[cfg(test)]
mod tests {
    use super::ViewportTracker;

    #[test]
    fn prefetch_range_clamps() {
        let tracker = ViewportTracker {
            first_visible_index: 10,
            last_visible_index: 25,
            prefetch_rows: 2,
            columns: 5,
        };

        let (start, end) = tracker.get_prefetch_range(100);
        assert_eq!(start, 0);
        assert_eq!(end, 36);
    }
}
