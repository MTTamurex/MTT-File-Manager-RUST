use crate::infrastructure::app_state_db::AppStateDb;
use eframe::egui;
use serde::{Deserialize, Serialize};

const PREFERENCE_KEY: &str = "settings_window_geometry";

/// Geometry in logical points, relative to the application's viewport.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SettingsWindowGeometry {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

impl SettingsWindowGeometry {
    pub fn from_rect(rect: egui::Rect) -> Option<Self> {
        let geometry = Self {
            x: rect.min.x,
            y: rect.min.y,
            width: rect.width(),
            height: rect.height(),
        };
        geometry.is_valid().then_some(geometry)
    }

    fn is_valid(self) -> bool {
        [self.x, self.y, self.width, self.height]
            .into_iter()
            .all(f32::is_finite)
            && self.width > 0.0
            && self.height > 0.0
            && (self.x + self.width).is_finite()
            && (self.y + self.height).is_finite()
    }

    pub fn rect(self) -> egui::Rect {
        egui::Rect::from_min_size(
            egui::pos2(self.x, self.y),
            egui::vec2(self.width, self.height),
        )
    }

    pub fn load(db: &AppStateDb) -> Option<Self> {
        let geometry: Self = serde_json::from_str(&db.get_preference(PREFERENCE_KEY)?).ok()?;
        geometry.is_valid().then_some(geometry)
    }

    pub fn preference(self) -> Option<(&'static str, String)> {
        self.is_valid()
            .then(|| serde_json::to_string(&self).ok())
            .flatten()
            .map(|value| (PREFERENCE_KEY, value))
    }
}

#[cfg(test)]
mod tests;
