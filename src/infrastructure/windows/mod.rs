//! Windows API modules
//! Follows .cursorrules: separation of concerns, < 300 lines per file

pub mod bitmap_conversion;
pub mod drives;
pub mod file_system;
pub mod formatting;
pub mod icons;
pub mod metadata;
pub mod shell_operations;
pub mod system_info;

// Re-export for convenience
pub use bitmap_conversion::*;
pub use drives::*;
pub use file_system::*;
pub use formatting::*;
pub use icons::*;
pub use metadata::*;
pub use shell_operations::*;
pub use system_info::*;
