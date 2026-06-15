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

/// File types whose icon must be extracted from the real file path because the
/// icon can vary per file.
#[inline]
pub fn is_per_file_icon_ext(ext: &str) -> bool {
    matches!(
        ext,
        "exe" | "lnk" | "ico" | "cur" | "ani" | "com" | "scr" | "url"
    )
}

/// File types that still share one icon per extension, but require a real file
/// path to seed that shared icon correctly.
#[inline]
pub fn requires_real_file_for_shared_icon(ext: &str) -> bool {
    matches!(canonical_icon_ext(ext), "dll")
}

pub use file_icons::{
    extract_drive_icon, extract_file_icon, extract_file_icon_by_path, extract_folder_icon,
    extract_icon_resource, get_file_type_icon,
};
pub use special::{extract_computer_icon, extract_recycle_bin_icon, extract_shell_icon};
pub use thumbnails::{
    extract_thumbnail, force_extract_folder_preview, force_extract_thumbnail,
    force_extract_thumbnail_with_size, get_folder_preview,
};
