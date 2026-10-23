use crate::domain::file_tag::FileTag;
use crate::domain::special_paths::{tag_id_from_view_path, COMPUTER_VIEW_ID};
use rustc_hash::FxHashMap;

pub(crate) fn tab_icon_style(
    path: &str,
    selected: bool,
    tag_definitions: &FxHashMap<i64, FileTag>,
) -> (&'static str, [u8; 4]) {
    if let Some(tag) = tag_id_from_view_path(path).and_then(|id| tag_definitions.get(&id)) {
        return ("tag", tag.color.to_color32().to_array());
    }

    let name = if path == COMPUTER_VIEW_ID {
        "home"
    } else {
        "folder"
    };
    let color = if selected {
        [30, 90, 180, 255]
    } else {
        [80, 80, 80, 255]
    };
    (name, color)
}
