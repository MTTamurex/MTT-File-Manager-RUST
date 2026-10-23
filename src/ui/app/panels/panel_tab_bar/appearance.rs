use eframe::egui;

use super::TAB_CORNER_RADIUS;

pub(super) fn layout_tab_title(
    ui: &egui::Ui,
    title: &str,
    font_id: egui::FontId,
    color: egui::Color32,
    max_width: f32,
) -> std::sync::Arc<egui::Galley> {
    let painter = ui.painter();
    let full_galley = painter.layout_no_wrap(title.to_owned(), font_id.clone(), color);
    if full_galley.size().x <= max_width {
        return full_galley;
    }

    let mut boundaries: Vec<usize> = title.char_indices().map(|(index, _)| index).collect();
    boundaries.push(title.len());
    let mut low = 0usize;
    let mut high = boundaries.len().saturating_sub(1);

    while low < high {
        let middle = (low + high).div_ceil(2);
        let mut candidate = title[..boundaries[middle]].to_owned();
        candidate.push('…');
        let galley = painter.layout_no_wrap(candidate, font_id.clone(), color);
        if galley.size().x <= max_width {
            low = middle;
        } else {
            high = middle - 1;
        }
    }

    let mut truncated = title[..boundaries[low]].to_owned();
    truncated.push('…');
    painter.layout_no_wrap(truncated, font_id, color)
}

pub(super) fn paint_tab_background(ui: &egui::Ui, rect: egui::Rect, color: egui::Color32) {
    ui.painter().rect_filled(
        rect,
        egui::CornerRadius {
            nw: TAB_CORNER_RADIUS,
            ne: TAB_CORNER_RADIUS,
            sw: 0,
            se: 0,
        },
        color,
    );

    if color.a() == u8::MAX {
        let body_top = (rect.min.y + TAB_CORNER_RADIUS as f32 - 1.0).min(rect.max.y);
        let body = egui::Rect::from_min_max(egui::pos2(rect.min.x, body_top), rect.max);
        ui.painter().rect_filled(body, 0.0, color);
    }
}

#[cfg(test)]
mod tests {
    use crate::domain::file_tag::{FileTag, TagColor};
    use crate::domain::special_paths::{tag_view_path, COMPUTER_VIEW_ID, RECYCLE_BIN_VIEW_ID};
    use crate::ui::tab_bar::tab_icon_style;
    use rustc_hash::FxHashMap;

    #[test]
    fn tag_tabs_use_tag_icons_in_the_current_tag_color() {
        let mut tags = FxHashMap::default();
        let path = tag_view_path(1);
        for color in TagColor::expanded_palette() {
            tags.insert(
                1,
                FileTag {
                    id: 1,
                    name: "Example".into(),
                    color,
                    position: 0,
                },
            );
            for selected in [false, true] {
                assert_eq!(
                    tab_icon_style(&path, selected, &tags),
                    ("tag", color.to_color32().to_array())
                );
            }
        }
    }

    #[test]
    fn other_tabs_and_missing_tags_keep_the_existing_style() {
        let tags = FxHashMap::default();
        for selected in [false, true] {
            let color = if selected {
                [30, 90, 180, 255]
            } else {
                [80, 80, 80, 255]
            };
            for path in ["folder", RECYCLE_BIN_VIEW_ID, &tag_view_path(1)] {
                assert_eq!(tab_icon_style(path, selected, &tags), ("folder", color));
            }
            assert_eq!(
                tab_icon_style(COMPUTER_VIEW_ID, selected, &tags),
                ("home", color)
            );
        }
    }

    #[test]
    fn filled_folder_asset_has_an_opaque_center() {
        let svg = crate::embedded_assets::get_icon("folder_filled").unwrap();
        let tree = usvg::Tree::from_data(svg, &usvg::Options::default()).unwrap();
        let mut pixmap = tiny_skia::Pixmap::new(24, 24).unwrap();
        resvg::render(
            &tree,
            tiny_skia::Transform::identity(),
            &mut pixmap.as_mut(),
        );
        assert_eq!(pixmap.pixel(12, 12).unwrap().alpha(), 255);
    }
}
