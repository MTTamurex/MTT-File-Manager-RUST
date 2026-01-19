//! Common helper functions for views
//! Follows .cursorrules: single responsibility, < 300 lines

use crate::domain::file_entry::FileEntry;
use crate::infrastructure::windows;

/// Gets file type string for display
pub fn get_file_type_string(item: &FileEntry) -> String {
    // Check for ZIP manually because is_dir might be true
    if item.name.to_lowercase().ends_with(".zip") {
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
