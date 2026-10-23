//! Installed font discovery and collection-aware loading, kept off the UI thread.

use eframe::egui;

pub(crate) const CUSTOM_FONT: &str = "custom_ui_font";

pub(crate) struct Catalog {
    database: fontdb::Database,
    pub families: Vec<String>,
}

impl Catalog {
    pub fn discover() -> Self {
        let mut database = fontdb::Database::new();
        #[cfg(not(windows))]
        database.load_system_fonts();
        #[cfg(windows)]
        {
            if let Some(root) =
                std::env::var_os("WINDIR").or_else(|| std::env::var_os("SYSTEMROOT"))
            {
                database.load_fonts_dir(std::path::PathBuf::from(root).join("Fonts"));
            }
            // Known-folder APIs respect redirected per-user data directories.
            for root in [dirs::data_local_dir(), dirs::data_dir()]
                .into_iter()
                .flatten()
            {
                database.load_fonts_dir(root.join("Microsoft/Windows/Fonts"));
            }
            super::registry::load_registered_fonts(&mut database);
        }
        Self::from_database(database)
    }

    pub(super) fn from_database(database: fontdb::Database) -> Self {
        let mut families: Vec<_> = database
            .faces()
            .flat_map(|face| face.families.iter().map(|(name, _)| name.clone()))
            .collect();
        families.sort_by_cached_key(|name| name.to_lowercase());
        families.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        Self { database, families }
    }

    pub fn apply(&self, fonts: &mut egui::FontDefinitions, family: &str) -> bool {
        if family.is_empty() {
            return true;
        }
        let Some(id) = self.database.query(&fontdb::Query {
            families: &[fontdb::Family::Name(family)],
            ..Default::default()
        }) else {
            return false;
        };
        let Some(data) = self
            .database
            .with_face_data(id, |bytes, index| super::hinted_font(bytes.to_vec(), index))
        else {
            return false;
        };
        install(fonts, data);
        true
    }
}

fn install(fonts: &mut egui::FontDefinitions, data: egui::FontData) {
    fonts
        .font_data
        .insert(CUSTOM_FONT.to_owned(), std::sync::Arc::new(data));
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, CUSTOM_FONT.to_owned());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundled_catalog() -> Catalog {
        let mut database = fontdb::Database::new();
        let fonts = egui::FontDefinitions::default();
        for data in fonts.font_data.values() {
            database.load_font_data(data.font.to_vec());
        }
        Catalog::from_database(database)
    }

    #[test]
    fn discovered_family_loads_with_original_fallbacks_and_hinting() {
        let catalog = bundled_catalog();
        assert!(!catalog.families.is_empty());
        let mut fonts = egui::FontDefinitions::default();
        let original = fonts.families[&egui::FontFamily::Proportional].clone();
        assert!(catalog.apply(&mut fonts, &catalog.families[0]));
        assert_eq!(
            &fonts.families[&egui::FontFamily::Proportional][1..],
            original
        );
        assert_eq!(fonts.font_data[CUSTOM_FONT].tweak.hinting, Some(true));
        // Exercise egui's parser/rasterizer, not just the discovery library.
        let ctx = egui::Context::default();
        ctx.set_fonts(fonts);
        let _ = ctx.run_ui(Default::default(), |ui| {
            ui.label("Sample text 0123");
        });
    }

    #[test]
    fn missing_family_does_not_change_fallbacks() {
        let catalog = Catalog {
            database: fontdb::Database::new(),
            families: Vec::new(),
        };
        let mut fonts = egui::FontDefinitions::default();
        let original = fonts.families.clone();
        assert!(!catalog.apply(&mut fonts, "Missing Test Family"));
        assert_eq!(fonts.families, original);
        assert!(catalog.apply(&mut fonts, ""));
        assert_eq!(fonts.families, original);
    }

    #[test]
    fn custom_font_only_overrides_proportional_text() {
        let mut fonts = egui::FontDefinitions::default();
        let icons = egui::FontFamily::Name("icons".into());
        fonts
            .families
            .insert(icons.clone(), vec!["icon_test".into()]);
        let original = fonts.families.clone();
        install(&mut fonts, egui::FontData::from_owned(Vec::new()));
        assert_eq!(
            fonts.families[&egui::FontFamily::Proportional][0],
            CUSTOM_FONT
        );
        assert_eq!(
            &fonts.families[&egui::FontFamily::Proportional][1..],
            original[&egui::FontFamily::Proportional]
        );
        assert_eq!(fonts.families[&icons], original[&icons]);
        assert_eq!(
            fonts.families[&egui::FontFamily::Monospace],
            original[&egui::FontFamily::Monospace]
        );
    }
}
