//! Per-family text scale. Font aliases isolate proportional UI text from icon
//! and monospace families, including when they share the same fallback bytes.

use eframe::egui;
use std::collections::HashMap;

pub(crate) const MIN: f32 = 0.75;
pub(crate) const MAX: f32 = 1.5;
pub(crate) const DEFAULT: f32 = 1.0;
const PREFIX: &str = "ui_font_scale:";

fn family_key(family: &str) -> String {
    if family.is_empty() {
        "segoe ui".into()
    } else {
        family.to_lowercase()
    }
}

pub(crate) fn preference_key(family: &str) -> String {
    format!("{PREFIX}{}", family_key(family))
}

pub(crate) fn normalize(scale: f32) -> f32 {
    if scale.is_finite() {
        scale.clamp(MIN, MAX)
    } else {
        DEFAULT
    }
}

pub(crate) fn from_preference(value: Option<&str>) -> f32 {
    normalize(
        value
            .and_then(|value| value.parse().ok())
            .unwrap_or(DEFAULT),
    )
}

#[derive(Default)]
pub(crate) struct FontSizes(HashMap<String, f32>);

impl FontSizes {
    pub fn load(db: &crate::infrastructure::app_state_db::AppStateDb) -> Self {
        Self(
            db.get_all_preferences()
                .into_iter()
                .filter_map(|(key, value)| {
                    key.strip_prefix(PREFIX)
                        .map(|family| (family.to_owned(), from_preference(Some(&value))))
                })
                .collect(),
        )
    }

    pub fn get(&self, family: &str) -> f32 {
        self.0.get(&family_key(family)).copied().unwrap_or(DEFAULT)
    }

    pub fn set(&mut self, family: &str, scale: f32) {
        self.0.insert(family_key(family), normalize(scale));
    }
}

/// Apply to an unscaled definition set. Never mutate the shared font data or
/// scale it in place repeatedly: that would also enlarge icons/monospace text
/// and compound the scale on successive adjustments.
pub(crate) fn apply(fonts: &mut egui::FontDefinitions, scale: f32) {
    let scale = normalize(scale);
    if scale == DEFAULT {
        return;
    }
    let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) else {
        return;
    };
    for name in family {
        let Some(data) = fonts.font_data.get(name) else {
            continue;
        };
        let mut data = (**data).clone();
        data.tweak.scale *= scale;
        let alias = format!("scaled_ui_{name}");
        fonts
            .font_data
            .insert(alias.clone(), std::sync::Arc::new(data));
        *name = alias;
    }
}

#[cfg(test)]
mod tests;
