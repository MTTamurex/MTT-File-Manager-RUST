//! Windows system-font loading shared by the main window and the standalone
//! viewer subprocesses.
//!
//! egui's bundled fonts only cover Latin/Cyrillic, so CJK text (and CJK file
//! names) render as tofu boxes unless the system fonts are loaded explicitly.
//! Windows Explorer relies on OS font fallback; egui does not.

use eframe::egui;

/// Loads the Windows system fonts into `fonts` and registers them as fallbacks
/// in the proportional and monospace families.
///
/// Adds Segoe UI / Segoe UI Symbol plus one font per major CJK locale, so
/// Simplified Chinese, Traditional Chinese, Japanese, and Korean render.
/// Best-effort: missing files are skipped, and `fonts` is left untouched when
/// the Windows font directory cannot be resolved.
pub fn install_system_fonts(fonts: &mut egui::FontDefinitions) {
    let Some(fonts_dir) = std::env::var_os("WINDIR")
        .map(std::path::PathBuf::from)
        .map(|windows_dir| windows_dir.join("Fonts"))
    else {
        return;
    };

    let mut loaded: Vec<String> = Vec::new();
    let mut load = |key: &str, file: &str, fonts: &mut egui::FontDefinitions| -> bool {
        if loaded.iter().any(|name| name == key) {
            return true;
        }
        if let Ok(data) = std::fs::read(fonts_dir.join(file)) {
            fonts.font_data.insert(
                key.to_owned(),
                std::sync::Arc::new(egui::FontData::from_owned(data)),
            );
            loaded.push(key.to_owned());
            return true;
        }
        false
    };

    load("segoe_ui", "segoeui.ttf", fonts);
    load("segoe_ui_symbol", "seguisym.ttf", fonts);

    // One font per CJK locale is enough; the first available file in each group
    // wins, so a machine without a given language pack degrades gracefully.
    for group in [
        &[("ms_yahei", "msyh.ttc")][..],
        &[("ms_jhenghei", "msjh.ttc")][..],
        &[("yu_gothic", "YuGothR.ttc"), ("ms_gothic", "msgothic.ttc")][..],
        &[("malgun", "malgun.ttf")][..],
    ] {
        for (key, file) in group {
            if load(key, file, fonts) {
                break;
            }
        }
    }

    if loaded.is_empty() {
        return;
    }

    if let Some(proportional) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
        if loaded.iter().any(|name| name == "segoe_ui") {
            proportional.insert(0, "segoe_ui".to_owned());
        }
        proportional.extend(
            loaded
                .iter()
                .filter(|name| name.as_str() != "segoe_ui")
                .cloned(),
        );
    }

    if let Some(monospace) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
        monospace.extend(loaded.clone());
    }
}
