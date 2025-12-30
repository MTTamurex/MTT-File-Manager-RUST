//! Common helper functions for views
//! Follows .cursorrules: single responsibility, < 300 lines

use crate::domain::file_entry::FileEntry;
use crate::infrastructure::windows_api as win_api;

/// Gets file type string for display
pub fn get_file_type_string(item: &FileEntry) -> String {
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
    win_api::format_date(timestamp)
}

/// Formats size for display
pub fn format_size(size: u64) -> String {
    win_api::format_size(size)
}

/// Opens file with shell
pub fn open_with_shell(path: &std::path::Path) {
    win_api::open_with_shell(path);
}
