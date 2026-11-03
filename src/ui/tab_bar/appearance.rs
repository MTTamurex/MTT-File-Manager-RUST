use crate::app::operations::tag_ops::tag_view_title;
use crate::domain::special_paths::{tag_id_from_view_path, COMPUTER_VIEW_ID, RECYCLE_BIN_VIEW_ID};
use crate::infrastructure::onedrive::special_folder_display_name;
use eframe::egui::{self, Color32, CornerRadius};
use rust_i18n::t;
use std::path::Path;

pub(super) fn display_title_for_tab_path(
    path: &str,
    fallback_title: &str,
    tag_definitions: &rustc_hash::FxHashMap<i64, crate::domain::file_tag::FileTag>,
) -> String {
    if path == COMPUTER_VIEW_ID {
        t!("nav.computer").to_string()
    } else if path == RECYCLE_BIN_VIEW_ID {
        t!("nav.recycle_bin").to_string()
    } else if let Some(tag_id) = tag_id_from_view_path(path) {
        tag_view_title(tag_id, tag_definitions)
    } else {
        special_folder_display_name(Path::new(path)).unwrap_or_else(|| fallback_title.to_string())
    }
}

pub(super) fn paint_tab_background(ui: &egui::Ui, rect: egui::Rect, color: Color32) {
    const RADIUS: u8 = 8;
    ui.painter().rect_filled(
        rect,
        CornerRadius {
            nw: RADIUS,
            ne: RADIUS,
            sw: 0,
            se: 0,
        },
        color,
    );
    // A second opaque pass fills the bottom corners without double-blending translucent fills.
    if color.a() == u8::MAX {
        let body_top = (rect.min.y + RADIUS as f32 - 1.0).min(rect.max.y);
        let body = egui::Rect::from_min_max(egui::pos2(rect.min.x, body_top), rect.max);
        ui.painter().rect_filled(body, 0.0, color);
    }
}

#[cfg(test)]
mod tests {
    use super::display_title_for_tab_path;
    use crate::domain::file_tag::{FileTag, TagColor};
    use crate::domain::special_paths::tag_view_path;
    use crate::tabs::TabManager;
    use rustc_hash::FxHashMap;

    fn definitions() -> FxHashMap<i64, FileTag> {
        let mut definitions = FxHashMap::default();
        definitions.insert(
            2,
            FileTag {
                id: 2,
                name: "Movies".to_string(),
                color: TagColor::Blue,
                position: 0,
                parent_id: None,
            },
        );
        definitions
    }

    #[test]
    fn background_tag_tab_displays_the_tag_name_without_activation() {
        let mut manager = TabManager::new();
        let index = manager
            .new_background_tab_at(&tag_view_path(2))
            .expect("tab cap not reached");
        let tab = &manager.tabs[index];

        let title = display_title_for_tab_path(&tab.path, &tab.title, &definitions());

        assert_eq!(tab.title, tag_view_path(2));
        assert_eq!(
            title,
            rust_i18n::t!("tags.filter_active", name = "Movies").to_string()
        );
    }

    #[test]
    fn missing_tag_definition_falls_back_to_the_id() {
        let title = display_title_for_tab_path(&tag_view_path(9), "::tag::9", &definitions());
        assert_eq!(
            title,
            rust_i18n::t!("tags.filter_active", name = "9").to_string()
        );
    }

    #[test]
    fn filesystem_paths_keep_the_fallback_title() {
        assert_eq!(
            display_title_for_tab_path(r"C:\Folder", "Folder", &definitions()),
            "Folder"
        );
    }
}
