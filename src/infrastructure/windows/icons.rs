//! Windows icon extraction functions
//! Follows .cursorrules: single responsibility, < 300 lines

mod file_icons;
mod special;
mod thumbnails;

pub use file_icons::{
    extract_drive_icon, extract_file_icon, extract_file_icon_by_path, extract_folder_icon,
    get_file_type_icon,
};
pub use special::{extract_computer_icon, extract_recycle_bin_icon, extract_shell_icon};
pub use thumbnails::{
    extract_thumbnail, force_extract_folder_preview, force_extract_thumbnail, get_folder_preview,
};
