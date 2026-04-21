rust_i18n::i18n!("locales", fallback = "pt-BR");

pub mod app; // NEW - Application state and initialization
pub mod application;
pub mod domain;
pub mod embedded_assets; // Embedded resources for portable executable
pub mod image_viewer;
pub mod image_viewer_minimal; // Minimal test viewer without complexity
pub mod infrastructure;
pub mod pdf_viewer;
pub mod tabs;
pub mod text_viewer;
pub mod ui;
pub mod viewer_runtime;
pub mod video_player;
pub mod workers;

// Re-export main app struct for easy access
pub use app::state::ImageViewerApp;
pub use infrastructure::threading::spawn_named;
