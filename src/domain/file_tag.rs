use eframe::egui::Color32;
use rustc_hash::FxHashMap;
use std::path::{Path, PathBuf};

pub fn normalize_tag_path_key(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

pub fn tag_ids_for_path<'a>(
    assignments: &'a FxHashMap<PathBuf, Vec<i64>>,
    path: &Path,
) -> Option<&'a [i64]> {
    if let Some(ids) = assignments.get(path) {
        return Some(ids.as_slice());
    }

    let path_key = normalize_tag_path_key(path);
    assignments.iter().find_map(|(assigned_path, ids)| {
        (normalize_tag_path_key(assigned_path) == path_key).then_some(ids.as_slice())
    })
}

pub fn path_has_tag(assignments: &FxHashMap<PathBuf, Vec<i64>>, path: &Path, tag_id: i64) -> bool {
    tag_ids_for_path(assignments, path).is_some_and(|ids| ids.contains(&tag_id))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TagColor {
    Red,
    Orange,
    Yellow,
    Green,
    Blue,
    Purple,
    Gray,
}

impl TagColor {
    pub const fn default_palette() -> [TagColor; 7] {
        [
            TagColor::Red,
            TagColor::Orange,
            TagColor::Yellow,
            TagColor::Green,
            TagColor::Blue,
            TagColor::Purple,
            TagColor::Gray,
        ]
    }

    pub const fn as_db_str(self) -> &'static str {
        match self {
            TagColor::Red => "red",
            TagColor::Orange => "orange",
            TagColor::Yellow => "yellow",
            TagColor::Green => "green",
            TagColor::Blue => "blue",
            TagColor::Purple => "purple",
            TagColor::Gray => "gray",
        }
    }

    pub fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "red" => Some(TagColor::Red),
            "orange" => Some(TagColor::Orange),
            "yellow" => Some(TagColor::Yellow),
            "green" => Some(TagColor::Green),
            "blue" => Some(TagColor::Blue),
            "purple" => Some(TagColor::Purple),
            "gray" => Some(TagColor::Gray),
            _ => None,
        }
    }

    pub const fn to_color32(self) -> Color32 {
        match self {
            TagColor::Red => Color32::from_rgb(255, 59, 48),
            TagColor::Orange => Color32::from_rgb(255, 149, 0),
            TagColor::Yellow => Color32::from_rgb(255, 204, 0),
            TagColor::Green => Color32::from_rgb(52, 199, 89),
            TagColor::Blue => Color32::from_rgb(0, 122, 255),
            TagColor::Purple => Color32::from_rgb(175, 82, 222),
            TagColor::Gray => Color32::from_rgb(142, 142, 147),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileTag {
    pub id: i64,
    pub name: String,
    pub color: TagColor,
    pub position: i64,
}

#[cfg(test)]
mod tests {
    use super::TagColor;

    #[test]
    fn tag_color_roundtrips_db_values() {
        for color in TagColor::default_palette() {
            assert_eq!(TagColor::from_db_str(color.as_db_str()), Some(color));
        }
    }
}
