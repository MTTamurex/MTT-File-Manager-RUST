//! Stable, location-specific manual widths. No filesystem access or inheritance.
use super::list_columns::ColumnId;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct WidthLocation(pub String);

impl WidthLocation {
    pub fn for_view(path: &str, computer: bool, recycle: bool) -> Option<Self> {
        if computer {
            return Some(Self("view:computer".into()));
        }
        if recycle {
            return Some(Self("view:recycle_bin".into()));
        }
        if path.is_empty() {
            return None;
        }
        Some(Self(format!(
            "location:{}",
            super::file_tag::normalize_tag_path_text(path).to_lowercase()
        )))
    }
}

pub(crate) fn valid_width(column: ColumnId, width: f32) -> bool {
    width.is_finite()
        && width
            >= if column == ColumnId::Name {
                100.0
            } else {
                // Matches the display clamp in the list view (fit_panel::min_width).
                64.0
            }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn locations_share_windows_path_aliases_but_not_parent_or_special_views() {
        let a = WidthLocation::for_view(r"C:\Example\Folder", false, false);
        assert_eq!(
            a,
            WidthLocation::for_view(r"\\?\c:\example\folder\", false, false)
        );
        assert_ne!(a, WidthLocation::for_view(r"C:\Example", false, false));
        assert_ne!(
            WidthLocation::for_view("", true, false),
            WidthLocation::for_view("", false, true)
        );
        assert_eq!(WidthLocation::for_view("", false, false), None);
    }
}
