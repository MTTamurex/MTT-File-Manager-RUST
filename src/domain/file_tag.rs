use eframe::egui::Color32;
use rustc_hash::{FxHashMap, FxHashSet};
use std::path::{Path, PathBuf};

pub fn normalize_tag_path_key(path: &Path) -> String {
    let mut normalized = normalize_tag_path_text(&path.to_string_lossy());
    if normalized.is_ascii() {
        normalized.make_ascii_lowercase();
        normalized
    } else {
        normalized.to_lowercase()
    }
}

pub fn normalize_tag_path_text(path: &str) -> String {
    let mut normalized = path.replace('/', "\\");
    if normalized
        .get(..8)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("\\\\?\\unc\\"))
    {
        normalized = format!("\\\\{}", &normalized[8..]);
    } else if normalized.starts_with("\\\\?\\")
        || normalized.starts_with("\\\\.\\")
        || normalized.starts_with("\\??\\")
    {
        normalized = normalized[4..].to_string();
    }

    while normalized.ends_with('\\')
        && !is_drive_root(&normalized)
        && !is_unc_share_root(&normalized)
    {
        normalized.pop();
    }
    normalized
}

fn is_drive_root(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() == 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\'
}

fn is_unc_share_root(path: &str) -> bool {
    if !path.starts_with("\\\\") {
        return false;
    }
    let mut parts = path.trim_matches('\\').split('\\');
    parts.next().is_some() && parts.next().is_some() && parts.next().is_none()
}

/// O(1) lookup of tag IDs assigned to `path`.
///
/// `assignments` must be a map keyed by the canonical normalized path
/// (lowercased, `\` separators, no trailing `\`) — see
/// [`build_tag_assignments_normalized`] and [`normalize_tag_path_key`].
///
/// Performs exactly one `String` allocation (the normalized key) per call,
/// independent of the assignments size. The previous O(N) case-insensitive
/// fallback was removed because grid view invokes this per visible item per
/// frame, and the fallback allocated an additional `String` per comparison.
pub fn tag_ids_for_path<'a>(
    assignments: &'a FxHashMap<String, Vec<i64>>,
    path: &Path,
) -> Option<&'a [i64]> {
    let key = normalize_tag_path_key(path);
    assignments.get(&key).map(|v| v.as_slice())
}

pub fn path_has_tag(assignments: &FxHashMap<String, Vec<i64>>, path: &Path, tag_id: i64) -> bool {
    tag_ids_for_path(assignments, path).is_some_and(|ids| ids.contains(&tag_id))
}

/// Compute checkbox states once per selection, not once per menu tag.
pub fn common_tag_ids(
    assignments: &FxHashMap<String, Vec<i64>>,
    paths: &[PathBuf],
) -> FxHashSet<i64> {
    let Some((first, rest)) = paths.split_first() else {
        return FxHashSet::default();
    };
    let mut common: FxHashSet<_> = tag_ids_for_path(assignments, first)
        .unwrap_or_default()
        .iter()
        .copied()
        .collect();
    for path in rest {
        if common.is_empty() {
            break;
        }
        let ids = tag_ids_for_path(assignments, path).unwrap_or_default();
        common.retain(|id| ids.contains(id));
    }
    common
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
    Pink,
    Brown,
    Mint,
    Teal,
    Cyan,
    Indigo,
    Lime,
    Olive,
    Black,
    Custom([u8; 3]),
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

    pub const fn expanded_palette() -> [TagColor; 16] {
        [
            TagColor::Red,
            TagColor::Orange,
            TagColor::Yellow,
            TagColor::Green,
            TagColor::Blue,
            TagColor::Purple,
            TagColor::Gray,
            TagColor::Pink,
            TagColor::Brown,
            TagColor::Mint,
            TagColor::Teal,
            TagColor::Cyan,
            TagColor::Indigo,
            TagColor::Lime,
            TagColor::Olive,
            TagColor::Black,
        ]
    }

    pub fn as_db_str(self) -> String {
        let name = match self {
            TagColor::Red => "red",
            TagColor::Orange => "orange",
            TagColor::Yellow => "yellow",
            TagColor::Green => "green",
            TagColor::Blue => "blue",
            TagColor::Purple => "purple",
            TagColor::Gray => "gray",
            TagColor::Pink => "pink",
            TagColor::Brown => "brown",
            TagColor::Mint => "mint",
            TagColor::Teal => "teal",
            TagColor::Cyan => "cyan",
            TagColor::Indigo => "indigo",
            TagColor::Lime => "lime",
            TagColor::Olive => "olive",
            TagColor::Black => "black",
            TagColor::Custom([r, g, b]) => return format!("#{r:02X}{g:02X}{b:02X}"),
        };
        name.to_owned()
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
            "pink" => Some(TagColor::Pink),
            "brown" => Some(TagColor::Brown),
            "mint" => Some(TagColor::Mint),
            "teal" => Some(TagColor::Teal),
            "cyan" => Some(TagColor::Cyan),
            "indigo" => Some(TagColor::Indigo),
            "lime" => Some(TagColor::Lime),
            "olive" => Some(TagColor::Olive),
            "black" => Some(TagColor::Black),
            _ => {
                let hex = value.strip_prefix('#')?;
                if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    return None;
                }
                Some(TagColor::Custom([
                    u8::from_str_radix(&hex[0..2], 16).ok()?,
                    u8::from_str_radix(&hex[2..4], 16).ok()?,
                    u8::from_str_radix(&hex[4..6], 16).ok()?,
                ]))
            }
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
            TagColor::Pink => Color32::from_rgb(255, 45, 85),
            TagColor::Brown => Color32::from_rgb(162, 132, 94),
            TagColor::Mint => Color32::from_rgb(0, 199, 190),
            TagColor::Teal => Color32::from_rgb(48, 176, 199),
            TagColor::Cyan => Color32::from_rgb(50, 173, 230),
            TagColor::Indigo => Color32::from_rgb(88, 86, 214),
            TagColor::Lime => Color32::from_rgb(191, 215, 48),
            TagColor::Olive => Color32::from_rgb(128, 128, 0),
            TagColor::Black => Color32::from_rgb(28, 28, 30),
            TagColor::Custom([r, g, b]) => Color32::from_rgb(r, g, b),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileTag {
    pub id: i64,
    pub name: String,
    pub color: TagColor,
    pub position: i64,
    /// Parent tag in the hierarchy; `None` for root tags.
    pub parent_id: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_checkboxes_use_the_intersection_and_handle_untagged_items() {
        let assignments = [
            (
                normalize_tag_path_key(Path::new(r"C:\Data\first.txt")),
                vec![1, 2],
            ),
            (
                normalize_tag_path_key(Path::new(r"C:\Data\second.txt")),
                vec![1, 3],
            ),
        ]
        .into_iter()
        .collect();
        let mut paths = vec![
            PathBuf::from(r"c:\data\FIRST.txt"),
            PathBuf::from(r"C:\Data\second.txt"),
        ];
        assert_eq!(
            common_tag_ids(&assignments, &paths),
            [1].into_iter().collect()
        );
        paths.push(PathBuf::from(r"C:\Data\untagged.txt"));
        assert!(common_tag_ids(&assignments, &paths).is_empty());
        assert!(common_tag_ids(&assignments, &[]).is_empty());
    }

    #[test]
    fn normalized_keys_preserve_unicode_and_windows_alias_equivalence() {
        assert_eq!(
            normalize_tag_path_key(Path::new(r"\\?\UNC\SERVER\Share\FÓLDER\FILE.TXT")),
            normalize_tag_path_key(Path::new(r"\\server\share\fólDer\file.txt"))
        );
        assert_eq!(
            normalize_tag_path_key(Path::new(r"\\?\C:\Data\FILE.TXT")),
            normalize_tag_path_key(Path::new(r"C:/Data/file.txt"))
        );
    }

    #[test]
    fn tag_color_roundtrips_db_values() {
        for color in TagColor::expanded_palette() {
            assert_eq!(TagColor::from_db_str(&color.as_db_str()), Some(color));
        }
    }

    #[test]
    fn custom_tag_colors_roundtrip_without_losing_rgb_values() {
        for rgb in [[0, 0, 0], [255, 255, 255], [12, 34, 56], [0, 128, 255]] {
            let color = TagColor::Custom(rgb);
            assert_eq!(TagColor::from_db_str(&color.as_db_str()), Some(color));
            assert_eq!(color.to_color32().to_array(), [rgb[0], rgb[1], rgb[2], 255]);
        }
        assert_eq!(TagColor::Custom([12, 34, 56]).as_db_str(), "#0C2238");
        assert_eq!(
            TagColor::from_db_str("#aBcDeF"),
            Some(TagColor::Custom([171, 205, 239]))
        );
    }

    #[test]
    fn malformed_custom_tag_colors_are_rejected() {
        for value in [
            "",
            "unknown",
            "123456",
            "#12345",
            "#1234567",
            "#12345678",
            "#GG0000",
            "#é0000",
            "#000000 ",
        ] {
            assert_eq!(TagColor::from_db_str(value), None, "{value}");
        }
    }

    #[test]
    fn default_palette_keeps_original_colors() {
        assert_eq!(
            TagColor::default_palette(),
            [
                TagColor::Red,
                TagColor::Orange,
                TagColor::Yellow,
                TagColor::Green,
                TagColor::Blue,
                TagColor::Purple,
                TagColor::Gray,
            ]
        );
    }
}
