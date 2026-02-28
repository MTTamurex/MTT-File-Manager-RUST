//! Windows icon extraction functions
//! Follows .cursorrules: single responsibility, < 300 lines

mod file_icons;
mod special;
mod thumbnails;

/// Returns the canonical extension for icon lookups.
/// Extensions that share the same Windows shell icon (e.g. .sys → .dll gear icon)
/// are mapped to a single canonical form so that caching, disk-persistence and
/// SHGetFileInfoW all operate on the same key.
#[inline]
pub fn canonical_icon_ext(ext: &str) -> &str {
    match ext {
        "sys" | "drv" | "ocx" => "dll",
        other => other,
    }
}

pub use file_icons::{
    extract_drive_icon, extract_file_icon, extract_file_icon_by_path, extract_folder_icon,
    get_file_type_icon,
};
pub use special::{extract_computer_icon, extract_recycle_bin_icon, extract_shell_icon};
pub use thumbnails::{
    extract_thumbnail, force_extract_folder_preview, force_extract_thumbnail, get_folder_preview,
};
