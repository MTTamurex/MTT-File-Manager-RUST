use eframe::egui;
use mtt_file_manager::app::ImageViewerApp;

/// Load application icon from embedded PNG bytes
fn load_app_icon() -> Option<egui::IconData> {
    // Load PNG from embedded bytes using image crate
    match image::load_from_memory(mtt_file_manager::embedded_assets::APP_ICON_PNG) {
        Ok(img) => {
            // Resize to 256x256 for optimal display (Windows icon standard)
            let resized = img.resize_exact(256, 256, image::imageops::FilterType::Lanczos3);
            let rgba_image = resized.to_rgba8();
            let pixels = rgba_image.into_raw();

            Some(egui::IconData {
                rgba: pixels,
                width: 256,
                height: 256,
            })
        }
        Err(e) => {
            eprintln!("Warning: Failed to load embedded app icon: {}", e);
            None
        }
    }
}

fn main() -> eframe::Result<()> {
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
            // Carrega Segoe UI (fonte do Windows Explorer) + Symbol para Unicode completo
            let mut fonts = egui::FontDefinitions::default();
            let mut loaded_fonts = Vec::new();

            // 1. Segoe UI (fonte principal)
            let segoe_path = std::path::PathBuf::from("C:\\Windows\\Fonts\\segoeui.ttf");
            if let Ok(font_data) = std::fs::read(&segoe_path) {
                fonts.font_data.insert(
                    "segoe_ui".to_owned(),
                    std::sync::Arc::new(egui::FontData::from_owned(font_data)),
                );
                loaded_fonts.push("segoe_ui".to_owned());
            }

            // 2. Segoe UI Symbol (fallback 1 - símbolos)
            let symbol_path = std::path::PathBuf::from("C:\\Windows\\Fonts\\seguisym.ttf");
            if let Ok(font_data) = std::fs::read(&symbol_path) {
                fonts.font_data.insert(
                    "segoe_ui_symbol".to_owned(),
                    std::sync::Arc::new(egui::FontData::from_owned(font_data)),
                );
                loaded_fonts.push("segoe_ui_symbol".to_owned());
            }

            // 3. Arial Unicode MS (fallback 2 - se disponível)
            let arial_path = std::path::PathBuf::from("C:\\Windows\\Fonts\\ARIALUNI.TTF");
            if let Ok(font_data) = std::fs::read(&arial_path) {
                fonts.font_data.insert(
                    "arial_unicode".to_owned(),
                    std::sync::Arc::new(egui::FontData::from_owned(font_data)),
                );
                loaded_fonts.push("arial_unicode".to_owned());
            }

            // 4. Remix Icon (Fonte de Ícones dedicada) - Embarcada no executável
            {
                let data = mtt_file_manager::embedded_assets::REMIXICON_TTF.to_vec();
                fonts.font_data.insert(
                    "remix_icon".to_owned(),
                    std::sync::Arc::new(egui::FontData::from_owned(data)),
                );

                // Definir uma família específica para ícones
                fonts.families.insert(
                    egui::FontFamily::Name("icons".into()),
                    vec!["remix_icon".to_owned()],
                );
            }

            // Adiciona apenas fontes carregadas
            if !loaded_fonts.is_empty() {
                fonts
                    .families
                    .get_mut(&egui::FontFamily::Proportional)
                    .unwrap()
                    .extend(loaded_fonts.clone());

                fonts
                    .families
                    .get_mut(&egui::FontFamily::Monospace)
                    .unwrap()
                    .extend(loaded_fonts.clone());
            }

            cc.egui_ctx.set_fonts(fonts);

            Ok(Box::new(ImageViewerApp::new(cc)))
        }),
    )
}
