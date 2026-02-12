use eframe::egui;
use mtt_file_manager::app::ImageViewerApp;

/// Load application icon from embedded PNG bytes
fn load_app_icon() -> Option<egui::IconData> {
    // Load PNG from embedded bytes using image crate
    match image::load_from_memory(mtt_file_manager::embedded_assets::APP_ICON_PNG) {
        Ok(img) => {
            // Resize to 256x256 for optimal display (Windows icon standard)
            let resized = img.resize_exact(256, 256, image::imageops::FilterType::CatmullRom);
            let rgba_image = resized.to_rgba8();
            let pixels = rgba_image.into_raw();

            Some(egui::IconData {
                rgba: pixels,
                width: 256,
                height: 256,
            })
        }
        Err(e) => {
            log::warn!("Failed to load embedded app icon: {}", e);
            None
        }
    }
}

fn main() -> eframe::Result<()> {
    // Initialize logging: default=warn, MTT modules=info, RUST_LOG env overrides
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn,mtt_file_manager=info"))
        .format_timestamp_millis()
        .init();

    log::info!("MTT File Manager starting");

    // Initialize codec name cache (queries Windows Registry on-demand)
    mtt_file_manager::infrastructure::windows::codec_registry::init_codec_cache();

    // Load application icon
    let icon_data = load_app_icon();

    // 3-STAGE STARTUP: Start hidden and small (NOT maximized here)
    let mut viewport = egui::ViewportBuilder::default()
        .with_visible(false) // Start hidden
        .with_maximized(false) // NOT maximized at creation
        .with_inner_size([800.0, 600.0]) // Small initial size (will be maximized in update)
        .with_title("MTT File Manager")
        .with_app_id("mtt-file-manager")
        .with_decorations(false) // Borderless window - resize handled by native subclass
        .with_resizable(true); // Enable resize (handled by WM_NCHITTEST subclass)

    // Set window icon if loaded successfully
    if let Some(icon) = icon_data {
        viewport = viewport.with_icon(icon);
    }

    let options = eframe::NativeOptions {
        viewport,
        persist_window: false, // Disable eframe persistence - we control manually
        ..Default::default()
    };

    eframe::run_native(
        "MTT File Manager",
        options,
        Box::new(|cc| {
            // STARTUP OPTIMIZATION: Fonts are now loaded asynchronously in app/init.rs
            // This allows the window to appear immediately with default fonts.
            // When the background thread finishes, the new fonts (Segoe UI) are applied dynamically.

            Ok(Box::new(ImageViewerApp::new(cc)))
        }),
    )
}
