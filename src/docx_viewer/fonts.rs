use std::collections::BTreeMap;
use std::path::PathBuf;

use docx_raster::FontChains;
use ooxml_text::font_store::{FontId, FontStore};
use serde_json::Value;

const WINDOWS_FONTS: &[(&str, &str, bool, bool)] = &[
    ("calibri.ttf", "Calibri", false, false),
    ("calibrib.ttf", "Calibri", true, false),
    ("calibrii.ttf", "Calibri", false, true),
    ("calibriz.ttf", "Calibri", true, true),
    ("arial.ttf", "Arial", false, false),
    ("arialbd.ttf", "Arial", true, false),
    ("ariali.ttf", "Arial", false, true),
    ("arialbi.ttf", "Arial", true, true),
    ("times.ttf", "Times New Roman", false, false),
    ("timesbd.ttf", "Times New Roman", true, false),
    ("timesi.ttf", "Times New Roman", false, true),
    ("timesbi.ttf", "Times New Roman", true, true),
    ("cour.ttf", "Courier New", false, false),
    ("courbd.ttf", "Courier New", true, false),
    ("couri.ttf", "Courier New", false, true),
    ("courbi.ttf", "Courier New", true, true),
    ("segoeui.ttf", "Segoe UI", false, false),
    ("segoeuib.ttf", "Segoe UI", true, false),
    ("segoeuii.ttf", "Segoe UI", false, true),
    ("segoeuiz.ttf", "Segoe UI", true, true),
    ("seguisym.ttf", "Segoe UI Symbol", false, false),
    ("malgun.ttf", "Malgun Gothic", false, false),
];

struct FontFace {
    family: String,
    bold: bool,
    italic: bool,
    id: u32,
}

pub(super) struct RegisteredFonts {
    pub store: FontStore,
    pub chains: FontChains,
    pub layout_chain_ids: BTreeMap<String, Vec<u32>>,
}

impl RegisteredFonts {
    pub fn load(requirements: &Value) -> Result<Self, String> {
        docx_layout::clear_measure_fonts();

        let fonts_dir = std::env::var_os("WINDIR")
            .map(PathBuf::from)
            .map(|windows_dir| windows_dir.join("Fonts"))
            .ok_or_else(|| rust_i18n::t!("docxviewer.fonts_unavailable").to_string())?;

        let mut store = FontStore::new();
        let mut faces = Vec::new();
        for (file, family, bold, italic) in WINDOWS_FONTS {
            let Ok(bytes) = std::fs::read(fonts_dir.join(file)) else {
                continue;
            };

            let mut validation_store = FontStore::new();
            if validation_store.register(bytes.clone()).is_err() {
                continue;
            }
            let Ok(layout_id) = docx_layout::register_measure_font(&bytes) else {
                continue;
            };
            let Ok(raster_id) = store.register(bytes) else {
                continue;
            };
            if layout_id != raster_id.to_u32() {
                return Err(rust_i18n::t!("docxviewer.font_registry_mismatch").to_string());
            }

            faces.push(FontFace {
                family: family.to_ascii_lowercase(),
                bold: *bold,
                italic: *italic,
                id: layout_id,
            });
        }

        if faces.is_empty() {
            return Err(rust_i18n::t!("docxviewer.fonts_unavailable").to_string());
        }

        let mut chains = FontChains::new();
        let mut layout_chain_ids = BTreeMap::new();
        let requirements = requirements
            .as_array()
            .ok_or_else(|| rust_i18n::t!("docxviewer.font_requirements_invalid").to_string())?;

        for requirement in requirements {
            let Some(key) = requirement.get("key").and_then(Value::as_str) else {
                continue;
            };
            let family = requirement
                .get("family")
                .and_then(Value::as_str)
                .unwrap_or("Calibri");
            let bold = requirement
                .get("bold")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let italic = requirement
                .get("italic")
                .and_then(Value::as_bool)
                .unwrap_or(false);

            let primary = select_face(&faces, family, bold, italic)
                .ok_or_else(|| rust_i18n::t!("docxviewer.fonts_unavailable").to_string())?;
            let mut chain = vec![primary];
            for script in requirement
                .get("scripts")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                let fallback = match script {
                    "arabic" => find_face(&faces, "segoe ui", false, false),
                    "hebrew" | "eastAsian" | "east_asian" => {
                        find_face(&faces, "microsoft yahei", false, false)
                            .or_else(|| find_face(&faces, "segoe ui", false, false))
                    }
                    _ => None,
                };
                if let Some(id) = fallback.filter(|id| !chain.contains(id)) {
                    chain.push(id);
                }
            }
            if let Some(symbol) =
                find_face(&faces, "segoe ui symbol", false, false).filter(|id| !chain.contains(id))
            {
                chain.push(symbol);
            }

            layout_chain_ids.insert(key.to_owned(), chain.clone());
            chains.insert(
                key.to_owned(),
                chain.into_iter().map(FontId::from_u32).collect(),
            );
        }

        if chains.is_empty() {
            let fallback = find_face(&faces, "calibri", false, false)
                .or_else(|| find_face(&faces, "arial", false, false))
                .unwrap_or(faces[0].id);
            let key = "calibri|0|0".to_owned();
            chains.insert(key.clone(), vec![FontId::from_u32(fallback)]);
            layout_chain_ids.insert(key, vec![fallback]);
        }

        Ok(Self {
            store,
            chains,
            layout_chain_ids,
        })
    }
}

fn select_face(faces: &[FontFace], family: &str, bold: bool, italic: bool) -> Option<u32> {
    let requested = family.to_ascii_lowercase();
    let candidates = if requested.contains("calibri") || requested.contains("carlito") {
        &["calibri", "arial", "segoe ui"][..]
    } else if requested.contains("cambria") || requested.contains("times") {
        &["times new roman", "cambria", "arial"][..]
    } else if requested.contains("courier") || requested.contains("consolas") {
        &["courier new", "segoe ui"][..]
    } else if requested.contains("yahei") || requested.contains("jhenghei") {
        &["microsoft yahei", "microsoft jhenghei", "segoe ui"][..]
    } else {
        &[requested.as_str(), "segoe ui", "arial"][..]
    };

    candidates
        .iter()
        .find_map(|family| find_face(faces, family, bold, italic))
}

fn find_face(faces: &[FontFace], family: &str, bold: bool, italic: bool) -> Option<u32> {
    faces
        .iter()
        .find(|face| face.family == family && face.bold == bold && face.italic == italic)
        .or_else(|| {
            faces
                .iter()
                .find(|face| face.family == family && face.bold == bold && !face.italic)
        })
        .or_else(|| {
            faces
                .iter()
                .find(|face| face.family == family && !face.bold && !face.italic)
        })
        .map(|face| face.id)
}
