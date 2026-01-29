//! Windows API modules
//! Follows .cursorrules: separation of concerns, < 300 lines per file

pub mod bitmap_conversion;
pub mod codec_registry;
pub mod device_change;
pub mod drives;
pub mod file_flags;
pub mod file_system;
pub mod file_type;
pub mod formatting;
pub mod icons;
pub mod iso_mount;
pub mod media_foundation;
pub mod metadata;
pub mod native_menu;
pub mod recycle_bin;
pub mod shell_folder;
pub mod shell_operations;
pub mod system_info;
pub mod window_subclass;


// Re-export for convenience
pub use bitmap_conversion::*;
pub use codec_registry::*;
pub use device_change::*;
pub use drives::*;
pub use file_flags::*;
pub use file_system::*;
pub use file_type::{
    find_folder_preview_item, get_perceived_type, is_audio_extension, is_image_extension,
    is_media_extension, is_video_extension, PerceivedType,
};
pub use formatting::*;
pub use icons::*;
pub use iso_mount::*;
pub use media_foundation::*;
pub use metadata::*;
pub use native_menu::*;
pub use recycle_bin::*;
pub use shell_folder::{is_shell_navigation_path, list_shell_folder};
pub use shell_operations::*;
pub use system_info::*;
pub use window_subclass::*;
