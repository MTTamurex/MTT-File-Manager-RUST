//! Persistent display options; unset colors retain theme-aware defaults.
use crate::application::ClipboardOp;
use eframe::egui::{self, Color32};

pub(crate) const PREFERENCE_KEY: &str = "clipboard_highlights";

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct HighlightPreferences {
    pub enabled: bool,
    pub copy_color: Option<[u8; 3]>,
    pub cut_color: Option<[u8; 3]>,
}

impl Default for HighlightPreferences {
    fn default() -> Self {
        Self {
            enabled: true,
            copy_color: None,
            cut_color: None,
        }
    }
}

impl HighlightPreferences {
    pub(crate) fn load(value: Option<&str>) -> Self {
        value
            .and_then(|value| serde_json::from_str(value).ok())
            .unwrap_or_default()
    }

    pub(crate) fn serialize(self) -> String {
        serde_json::to_string(&self).expect("highlight preferences are JSON serializable")
    }

    pub(crate) fn color(self, operation: ClipboardOp, dark: bool) -> Color32 {
        let custom = match operation {
            ClipboardOp::Copy => self.copy_color,
            ClipboardOp::Move => self.cut_color,
        };
        let [r, g, b] = custom.unwrap_or(match (operation, dark) {
            (ClipboardOp::Copy, false) => [25, 125, 65],
            (ClipboardOp::Copy, true) => [95, 205, 135],
            (ClipboardOp::Move, false) => [170, 105, 0],
            (ClipboardOp::Move, true) => [235, 180, 65],
        });
        Color32::from_rgb(r, g, b)
    }
}

pub(crate) fn remember(ctx: &egui::Context, preferences: HighlightPreferences) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::new(PREFERENCE_KEY), preferences));
    ctx.request_repaint();
}

pub(crate) fn current(ctx: &egui::Context) -> HighlightPreferences {
    ctx.data(|data| data.get_temp(egui::Id::new(PREFERENCE_KEY)))
        .unwrap_or_default()
}
