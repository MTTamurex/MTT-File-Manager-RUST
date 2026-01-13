pub mod app; // NEW - Application state and initialization
pub mod application;
pub mod domain;
pub mod infrastructure;
pub mod tabs;
pub mod ui;
pub mod workers;

// Re-export main app struct for easy access
pub use app::state::ImageViewerApp;

pub use ui::components::item_slot::draw_custom_folder;
