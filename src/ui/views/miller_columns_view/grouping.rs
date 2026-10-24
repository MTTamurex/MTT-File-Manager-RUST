//! Geometry shared by rendering, scrolling and rectangle selection in a column.

use std::sync::Arc;

use eframe::egui;

use crate::application::grouping::{GroupKey, GroupProjection};
use crate::ui::cache::FxHashSet;
use crate::ui::views::group_header::{GROUP_GAP, GROUP_HEADER_HEIGHT};
use crate::ui::views::rectangle_selection::{
    GroupedProjectionIdentity, GroupedRectangleLayout, GroupedRectangleMetrics,
    GroupedRectangleSection, RectangleSelectionMetrics, RectangleSelectionView,
};

pub(super) struct ColumnSection {
    pub key: GroupKey,
    pub header_y: f32,
    pub item_indices: Arc<[usize]>,
    pub collapsed: bool,
}

pub(super) struct ColumnGroups {
    pub sections: Vec<ColumnSection>,
    pub content_height: f32,
}

impl ColumnGroups {
    pub fn new(
        projection: &GroupProjection,
        collapsed: &FxHashSet<GroupKey>,
        row_height: f32,
    ) -> Self {
        let mut content_height = 0.0;
        let sections = projection
            .sections
            .iter()
            .map(|section| {
                let header_y = content_height;
                let collapsed = collapsed.contains(&section.key);
                content_height += GROUP_HEADER_HEIGHT + GROUP_GAP;
                if !collapsed {
                    content_height += section.item_indices.len() as f32 * row_height;
                }
                ColumnSection {
                    key: section.key.clone(),
                    header_y,
                    item_indices: section.item_indices.clone(),
                    collapsed,
                }
            })
            .collect();
        Self {
            sections,
            content_height,
        }
    }

    pub fn rectangle_metrics(
        &self,
        item_count: usize,
        row_height: f32,
        width: f32,
    ) -> RectangleSelectionMetrics {
        let sections: Arc<[_]> = self
            .sections
            .iter()
            .filter(|section| !section.collapsed)
            .map(|section| GroupedRectangleSection {
                item_indices: section.item_indices.clone(),
                origin: egui::pos2(0.0, section.header_y + GROUP_HEADER_HEIGHT),
            })
            .collect::<Vec<_>>()
            .into();
        RectangleSelectionMetrics::Grouped(GroupedRectangleMetrics {
            view: RectangleSelectionView::List,
            projection_identity: GroupedProjectionIdentity::new(sections.clone(), item_count),
            sections,
            layout: GroupedRectangleLayout::List { row_height },
            item_count,
            content_width: width,
            content_height: self.content_height,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::grouping::{GroupSection, NameGroup};
    use crate::ui::views::rectangle_selection::collect_indices_in_rect;

    #[test]
    fn headers_and_collapsed_groups_are_not_selectable() {
        let first = GroupKey::Name(NameGroup::AToH);
        let second = GroupKey::Name(NameGroup::IToP);
        let projection = GroupProjection {
            sections: vec![
                GroupSection {
                    key: first,
                    item_indices: vec![2, 0].into(),
                },
                GroupSection {
                    key: second.clone(),
                    item_indices: vec![1].into(),
                },
            ],
        };
        let groups = ColumnGroups::new(&projection, &FxHashSet::from_iter([second]), 24.0);
        assert_eq!(
            groups.content_height,
            2.0 * (GROUP_HEADER_HEIGHT + GROUP_GAP) + 48.0
        );
        let metrics = groups.rectangle_metrics(3, 24.0, 200.0);
        let header = egui::Rect::from_min_max(
            egui::pos2(0.0, 0.0),
            egui::pos2(200.0, GROUP_HEADER_HEIGHT - 1.0),
        );
        assert!(collect_indices_in_rect(header, metrics.clone()).is_empty());
        let all =
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, groups.content_height));
        assert_eq!(
            collect_indices_in_rect(all, metrics.clone()),
            FxHashSet::from_iter([0, 2])
        );
        let RectangleSelectionMetrics::Grouped(metrics) = metrics else {
            panic!("expected grouped geometry")
        };
        assert_eq!(metrics.visual_order().collect::<Vec<_>>(), vec![2, 0]);
    }
}
