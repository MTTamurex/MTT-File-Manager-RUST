use eframe::egui::{self, Color32, Pos2, Rect, Stroke};
use std::path::PathBuf;
use std::sync::Arc;

use crate::ui::cache::FxHashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RectangleSelectionView {
    Grid,
    List,
    ColumnList,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum RectangleSelectionSource {
    #[default]
    CurrentItems,
    MillerAncestor {
        directory: PathBuf,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RectangleSelectionModifiers {
    pub ctrl: bool,
    pub shift: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct GridRectangleMetrics {
    pub count: usize,
    pub cols: usize,
    pub padding: f32,
    pub item_w: f32,
    pub item_h: f32,
    pub virtual_cell_h: f32,
    pub content_width: f32,
    pub content_height: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct ListRectangleMetrics {
    pub count: usize,
    pub row_height: f32,
    pub content_width: f32,
    pub content_height: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct ColumnListRectangleMetrics {
    pub count: usize,
    pub rows_per_column: usize,
    pub column_width: f32,
    pub row_height: f32,
    pub content_width: f32,
    pub content_height: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GroupedRectangleSection {
    pub item_indices: Arc<[usize]>,
    pub origin: Pos2,
}

#[derive(Clone, Debug)]
pub struct GroupedProjectionIdentity {
    sections: Arc<[GroupedRectangleSection]>,
    item_count: usize,
}

impl GroupedProjectionIdentity {
    pub fn new(sections: Arc<[GroupedRectangleSection]>, item_count: usize) -> Self {
        Self {
            sections,
            item_count,
        }
    }
}

impl PartialEq for GroupedProjectionIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.item_count == other.item_count
            && self.sections.len() == other.sections.len()
            && self
                .sections
                .iter()
                .zip(other.sections.iter())
                .all(|(left, right)| Arc::ptr_eq(&left.item_indices, &right.item_indices))
    }
}

impl Eq for GroupedProjectionIdentity {}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GroupedRectangleLayout {
    Grid {
        cols: usize,
        item_w: f32,
        item_h: f32,
        column_step: f32,
        row_step: f32,
    },
    List {
        row_height: f32,
    },
    ColumnList {
        rows_per_column: usize,
        column_width: f32,
        row_height: f32,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct GroupedRectangleMetrics {
    pub view: RectangleSelectionView,
    pub sections: Arc<[GroupedRectangleSection]>,
    pub projection_identity: GroupedProjectionIdentity,
    pub layout: GroupedRectangleLayout,
    pub item_count: usize,
    pub content_width: f32,
    pub content_height: f32,
}

impl GroupedRectangleMetrics {
    pub fn visual_order(&self) -> impl Iterator<Item = usize> + '_ {
        self.sections
            .iter()
            .flat_map(|section| section.item_indices.iter().copied())
            .filter(|index| *index < self.item_count)
    }
}

#[derive(Clone, Debug)]
pub enum RectangleSelectionMetrics {
    Grid(GridRectangleMetrics),
    List(ListRectangleMetrics),
    ColumnList(ColumnListRectangleMetrics),
    Grouped(GroupedRectangleMetrics),
}

impl RectangleSelectionMetrics {
    pub fn view(&self) -> RectangleSelectionView {
        match self {
            Self::Grid(_) => RectangleSelectionView::Grid,
            Self::List(_) => RectangleSelectionView::List,
            Self::ColumnList(_) => RectangleSelectionView::ColumnList,
            Self::Grouped(metrics) => metrics.view,
        }
    }

    pub fn content_width(&self) -> f32 {
        match self {
            Self::Grid(metrics) => metrics.content_width,
            Self::List(metrics) => metrics.content_width,
            Self::ColumnList(metrics) => metrics.content_width,
            Self::Grouped(metrics) => metrics.content_width,
        }
    }

    pub fn content_height(&self) -> f32 {
        match self {
            Self::Grid(metrics) => metrics.content_height,
            Self::List(metrics) => metrics.content_height,
            Self::ColumnList(metrics) => metrics.content_height,
            Self::Grouped(metrics) => metrics.content_height,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RectangleSelectionState {
    pub view: RectangleSelectionView,
    pub source: RectangleSelectionSource,
    pub anchor_content: Pos2,
    pub current_content: Pos2,
    pub base_selection: FxHashSet<PathBuf>,
    pub base_preview_indices: FxHashSet<usize>,
    pub hit_indices: FxHashSet<usize>,
    pub preview_indices: FxHashSet<usize>,
    pub visual_order: Option<std::sync::Arc<[usize]>>,
    pub visual_order_source: Option<GroupedProjectionIdentity>,
    pub modifiers: RectangleSelectionModifiers,
    pub generation: usize,
}

impl RectangleSelectionState {
    pub fn new(
        view: RectangleSelectionView,
        anchor_content: Pos2,
        base_selection: FxHashSet<PathBuf>,
        base_preview_indices: FxHashSet<usize>,
        modifiers: RectangleSelectionModifiers,
        generation: usize,
    ) -> Self {
        Self::new_for_source(
            view,
            RectangleSelectionSource::CurrentItems,
            anchor_content,
            base_selection,
            base_preview_indices,
            modifiers,
            generation,
        )
    }

    pub fn new_for_source(
        view: RectangleSelectionView,
        source: RectangleSelectionSource,
        anchor_content: Pos2,
        base_selection: FxHashSet<PathBuf>,
        base_preview_indices: FxHashSet<usize>,
        modifiers: RectangleSelectionModifiers,
        generation: usize,
    ) -> Self {
        Self {
            view,
            source,
            anchor_content,
            current_content: anchor_content,
            base_selection,
            base_preview_indices,
            hit_indices: FxHashSet::default(),
            preview_indices: FxHashSet::default(),
            visual_order: None,
            visual_order_source: None,
            modifiers,
            generation,
        }
    }

    pub fn content_rect(&self) -> Rect {
        Rect::from_min_max(
            egui::pos2(
                self.anchor_content.x.min(self.current_content.x),
                self.anchor_content.y.min(self.current_content.y),
            ),
            egui::pos2(
                self.anchor_content.x.max(self.current_content.x),
                self.anchor_content.y.max(self.current_content.y),
            ),
        )
    }

    pub fn preview_contains(&self, index: usize) -> bool {
        self.preview_indices.contains(&index)
    }
}

#[derive(Clone, Debug, Default)]
pub struct RectangleSelectionFrame {
    pub source: RectangleSelectionSource,
    pub viewport_rect: Option<Rect>,
    pub current_scroll_y: f32,
    pub max_scroll_y: f32,
    pub current_scroll_x: f32,
    pub max_scroll_x: f32,
    pub metrics: Option<RectangleSelectionMetrics>,
    pub start_screen_pos: Option<Pos2>,
}

impl RectangleSelectionFrame {
    pub fn begin(
        &mut self,
        viewport_rect: Rect,
        current_scroll_x: f32,
        current_scroll_y: f32,
        max_scroll_x: f32,
        max_scroll_y: f32,
        metrics: Option<RectangleSelectionMetrics>,
    ) {
        self.source = RectangleSelectionSource::CurrentItems;
        self.begin_geometry(
            viewport_rect,
            current_scroll_x,
            current_scroll_y,
            max_scroll_x,
            max_scroll_y,
            metrics,
        );
    }

    fn begin_geometry(
        &mut self,
        viewport_rect: Rect,
        current_scroll_x: f32,
        current_scroll_y: f32,
        max_scroll_x: f32,
        max_scroll_y: f32,
        metrics: Option<RectangleSelectionMetrics>,
    ) {
        self.viewport_rect = Some(viewport_rect);
        self.current_scroll_y = current_scroll_y;
        self.max_scroll_y = max_scroll_y;
        self.current_scroll_x = current_scroll_x;
        self.max_scroll_x = max_scroll_x;
        self.metrics = metrics;
        self.start_screen_pos = None;
    }

    pub fn request_start(&mut self, screen_pos: Pos2) {
        if self.start_screen_pos.is_none() {
            self.start_screen_pos = Some(screen_pos);
        }
    }

    pub fn screen_to_content(&self, screen_pos: Pos2) -> Option<Pos2> {
        let viewport = self.viewport_rect?;
        let metrics = self.metrics.as_ref()?;
        let x = (screen_pos.x - viewport.left() + self.current_scroll_x)
            .clamp(0.0, metrics.content_width().max(0.0));
        let y = (screen_pos.y - viewport.top() + self.current_scroll_y)
            .clamp(0.0, metrics.content_height().max(0.0));
        Some(egui::pos2(x, y))
    }
}

pub fn collect_indices_in_rect(
    selection_rect: Rect,
    metrics: RectangleSelectionMetrics,
) -> FxHashSet<usize> {
    match metrics {
        RectangleSelectionMetrics::Grid(metrics) => collect_grid_indices(selection_rect, metrics),
        RectangleSelectionMetrics::List(metrics) => collect_list_indices(selection_rect, metrics),
        RectangleSelectionMetrics::ColumnList(metrics) => {
            collect_column_list_indices(selection_rect, metrics)
        }
        RectangleSelectionMetrics::Grouped(metrics) => {
            collect_grouped_indices(selection_rect, &metrics)
        }
    }
}

fn collect_grouped_indices(
    selection_rect: Rect,
    metrics: &GroupedRectangleMetrics,
) -> FxHashSet<usize> {
    let mut indices = FxHashSet::default();
    match metrics.layout {
        GroupedRectangleLayout::Grid {
            cols,
            item_w,
            item_h,
            column_step,
            row_step,
        } => {
            if cols == 0 || item_w <= 0.0 || item_h <= 0.0 || column_step <= 0.0 || row_step <= 0.0
            {
                return indices;
            }
            for section in metrics.sections.iter() {
                let rows = section.item_indices.len().div_ceil(cols);
                let Some(row_range) = intersecting_slots(
                    selection_rect.top(),
                    selection_rect.bottom(),
                    section.origin.y,
                    row_step,
                    item_h,
                    rows,
                ) else {
                    continue;
                };
                let Some(column_range) = intersecting_slots(
                    selection_rect.left(),
                    selection_rect.right(),
                    section.origin.x,
                    column_step,
                    item_w,
                    cols,
                ) else {
                    continue;
                };
                for row in row_range {
                    for column in column_range.clone() {
                        let position = row * cols + column;
                        let Some(&index) = section.item_indices.get(position) else {
                            break;
                        };
                        if index >= metrics.item_count {
                            continue;
                        }
                        let item_rect = Rect::from_min_size(
                            egui::pos2(
                                section.origin.x + column as f32 * column_step,
                                section.origin.y + row as f32 * row_step,
                            ),
                            egui::vec2(item_w, item_h),
                        );
                        if rects_intersect(selection_rect, item_rect) {
                            indices.insert(index);
                        }
                    }
                }
            }
        }
        GroupedRectangleLayout::List { row_height } => {
            if row_height <= 0.0
                || selection_rect.right() <= 0.0
                || selection_rect.left() >= metrics.content_width
            {
                return indices;
            }
            for section in metrics.sections.iter() {
                let Some(rows) = intersecting_slots(
                    selection_rect.top(),
                    selection_rect.bottom(),
                    section.origin.y,
                    row_height,
                    row_height,
                    section.item_indices.len(),
                ) else {
                    continue;
                };
                for position in rows {
                    let index = section.item_indices[position];
                    if index >= metrics.item_count {
                        continue;
                    }
                    let row_rect = Rect::from_min_size(
                        egui::pos2(
                            section.origin.x,
                            section.origin.y + position as f32 * row_height,
                        ),
                        egui::vec2(metrics.content_width, row_height),
                    );
                    if rects_intersect(selection_rect, row_rect) {
                        indices.insert(index);
                    }
                }
            }
        }
        GroupedRectangleLayout::ColumnList {
            rows_per_column,
            column_width,
            row_height,
        } => {
            if rows_per_column == 0 || column_width <= 0.0 || row_height <= 0.0 {
                return indices;
            }
            for section in metrics.sections.iter() {
                let columns = section.item_indices.len().div_ceil(rows_per_column);
                let Some(column_range) = intersecting_slots(
                    selection_rect.left(),
                    selection_rect.right(),
                    section.origin.x,
                    column_width,
                    column_width,
                    columns,
                ) else {
                    continue;
                };
                let Some(row_range) = intersecting_slots(
                    selection_rect.top(),
                    selection_rect.bottom(),
                    section.origin.y,
                    row_height,
                    row_height,
                    rows_per_column,
                ) else {
                    continue;
                };
                for column in column_range {
                    for row in row_range.clone() {
                        let position = column * rows_per_column + row;
                        let Some(&index) = section.item_indices.get(position) else {
                            break;
                        };
                        if index >= metrics.item_count {
                            continue;
                        }
                        let item_rect = Rect::from_min_size(
                            egui::pos2(
                                section.origin.x + column as f32 * column_width,
                                section.origin.y + row as f32 * row_height,
                            ),
                            egui::vec2(column_width, row_height),
                        );
                        if rects_intersect(selection_rect, item_rect) {
                            indices.insert(index);
                        }
                    }
                }
            }
        }
    }
    indices
}

fn intersecting_slots(
    selection_min: f32,
    selection_max: f32,
    origin: f32,
    step: f32,
    extent: f32,
    count: usize,
) -> Option<std::ops::RangeInclusive<usize>> {
    if count == 0
        || step <= 0.0
        || extent <= 0.0
        || selection_max <= origin
        || selection_min >= origin + (count - 1) as f32 * step + extent
    {
        return None;
    }
    let first = ((selection_min - origin) / step).floor().max(0.0) as usize;
    let last = ((selection_max - origin) / step).floor().max(0.0) as usize;
    Some(first.min(count - 1)..=last.min(count - 1))
}

fn collect_column_list_indices(
    selection_rect: Rect,
    metrics: ColumnListRectangleMetrics,
) -> FxHashSet<usize> {
    let mut indices = FxHashSet::default();
    if metrics.count == 0
        || metrics.rows_per_column == 0
        || metrics.column_width <= 0.0
        || metrics.row_height <= 0.0
    {
        return indices;
    }

    let column_count = metrics.count.div_ceil(metrics.rows_per_column);
    let first_col = (selection_rect.left() / metrics.column_width)
        .floor()
        .max(0.0) as usize;
    let last_col = (selection_rect.right() / metrics.column_width)
        .floor()
        .max(0.0) as usize;
    let last_col = last_col.min(column_count.saturating_sub(1));
    let first_row = (selection_rect.top() / metrics.row_height).floor().max(0.0) as usize;
    let last_row = (selection_rect.bottom() / metrics.row_height)
        .floor()
        .max(0.0) as usize;
    let last_row = last_row.min(metrics.rows_per_column.saturating_sub(1));

    for col in first_col..=last_col {
        for row in first_row..=last_row {
            let index = col * metrics.rows_per_column + row;
            if index >= metrics.count {
                break;
            }
            let item_rect = Rect::from_min_size(
                egui::pos2(
                    col as f32 * metrics.column_width,
                    row as f32 * metrics.row_height,
                ),
                egui::vec2(metrics.column_width, metrics.row_height),
            );
            if rects_intersect(selection_rect, item_rect) {
                indices.insert(index);
            }
        }
    }

    indices
}

fn collect_grid_indices(selection_rect: Rect, metrics: GridRectangleMetrics) -> FxHashSet<usize> {
    let mut indices = FxHashSet::default();
    if metrics.count == 0 || metrics.cols == 0 || metrics.virtual_cell_h <= 0.0 {
        return indices;
    }

    let total_rows = metrics.count.div_ceil(metrics.cols);
    if total_rows == 0 {
        return indices;
    }

    let first_row = ((selection_rect.top() - metrics.padding - metrics.item_h)
        / metrics.virtual_cell_h)
        .floor()
        .max(0.0) as usize;
    let last_row = ((selection_rect.bottom() - metrics.padding) / metrics.virtual_cell_h)
        .floor()
        .max(0.0) as usize;
    let last_row = last_row.min(total_rows.saturating_sub(1));

    for row in first_row..=last_row {
        for col in 0..metrics.cols {
            let index = row * metrics.cols + col;
            if index >= metrics.count {
                break;
            }

            let item_rect = Rect::from_min_size(
                egui::pos2(
                    metrics.padding + col as f32 * (metrics.item_w + metrics.padding),
                    metrics.padding + row as f32 * metrics.virtual_cell_h,
                ),
                egui::vec2(metrics.item_w, metrics.item_h),
            );
            if rects_intersect(selection_rect, item_rect) {
                indices.insert(index);
            }
        }
    }

    indices
}

fn collect_list_indices(selection_rect: Rect, metrics: ListRectangleMetrics) -> FxHashSet<usize> {
    let mut indices = FxHashSet::default();
    if metrics.count == 0 || metrics.row_height <= 0.0 {
        return indices;
    }

    let first_row = (selection_rect.top() / metrics.row_height).floor().max(0.0) as usize;
    let last_row = (selection_rect.bottom() / metrics.row_height)
        .floor()
        .max(0.0) as usize;
    let last_row = last_row.min(metrics.count.saturating_sub(1));

    for index in first_row..=last_row {
        let row_rect = Rect::from_min_size(
            egui::pos2(0.0, index as f32 * metrics.row_height),
            egui::vec2(metrics.content_width, metrics.row_height),
        );
        if rects_intersect(selection_rect, row_rect) {
            indices.insert(index);
        }
    }

    indices
}

fn rects_intersect(a: Rect, b: Rect) -> bool {
    a.min.x < b.max.x && b.min.x < a.max.x && a.min.y < b.max.y && b.min.y < a.max.y
}

pub fn paint_overlay(
    ui: &egui::Ui,
    state: &RectangleSelectionState,
    viewport_rect: Rect,
    current_scroll_x: f32,
    current_scroll_y: f32,
) {
    let content_rect = state.content_rect();
    let screen_rect = Rect::from_min_max(
        egui::pos2(
            viewport_rect.left() + content_rect.left() - current_scroll_x,
            viewport_rect.top() + content_rect.top() - current_scroll_y,
        ),
        egui::pos2(
            viewport_rect.left() + content_rect.right() - current_scroll_x,
            viewport_rect.top() + content_rect.bottom() - current_scroll_y,
        ),
    )
    .intersect(viewport_rect);

    if screen_rect.width() <= 1.0 || screen_rect.height() <= 1.0 {
        return;
    }

    let fill = Color32::from_rgba_unmultiplied(24, 122, 255, 28);
    let stroke = Color32::from_rgba_unmultiplied(24, 122, 255, 190);
    ui.painter().rect_filled(screen_rect, 0.0, fill);
    ui.painter().rect_stroke(
        screen_rect,
        0.0,
        Stroke::new(1.0, stroke),
        egui::StrokeKind::Inside,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(indices: FxHashSet<usize>) -> Vec<usize> {
        let mut values: Vec<_> = indices.into_iter().collect();
        values.sort_unstable();
        values
    }

    #[test]
    fn list_selection_does_not_include_row_that_only_touches_edge() {
        let indices = collect_indices_in_rect(
            Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(100.0, 20.0)),
            RectangleSelectionMetrics::List(ListRectangleMetrics {
                count: 3,
                row_height: 20.0,
                content_width: 100.0,
                content_height: 60.0,
            }),
        );

        assert_eq!(sorted(indices), vec![0]);
    }

    #[test]
    fn grid_selection_does_not_include_item_that_only_touches_edge() {
        let indices = collect_indices_in_rect(
            Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(10.0, 10.0)),
            RectangleSelectionMetrics::Grid(GridRectangleMetrics {
                count: 4,
                cols: 2,
                padding: 0.0,
                item_w: 10.0,
                item_h: 10.0,
                virtual_cell_h: 10.0,
                content_width: 20.0,
                content_height: 20.0,
            }),
        );

        assert_eq!(sorted(indices), vec![0]);
    }

    #[test]
    fn column_list_selection_maps_columns_top_to_bottom() {
        let indices = collect_indices_in_rect(
            Rect::from_min_max(egui::pos2(100.0, 20.0), egui::pos2(200.0, 60.0)),
            RectangleSelectionMetrics::ColumnList(ColumnListRectangleMetrics {
                count: 8,
                rows_per_column: 3,
                column_width: 100.0,
                row_height: 20.0,
                content_width: 300.0,
                content_height: 60.0,
            }),
        );

        assert_eq!(sorted(indices), vec![4, 5]);
    }

    #[test]
    fn grouped_selection_returns_logical_indices_and_ignores_headers() {
        let sections: Arc<[GroupedRectangleSection]> = vec![GroupedRectangleSection {
            item_indices: vec![7, 2].into(),
            origin: egui::pos2(0.0, 30.0),
        }]
        .into();
        let metrics = RectangleSelectionMetrics::Grouped(GroupedRectangleMetrics {
            view: RectangleSelectionView::List,
            projection_identity: GroupedProjectionIdentity::new(sections.clone(), 8),
            sections,
            layout: GroupedRectangleLayout::List { row_height: 20.0 },
            item_count: 8,
            content_width: 100.0,
            content_height: 70.0,
        });

        assert!(collect_indices_in_rect(
            Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(100.0, 29.0)),
            metrics.clone(),
        )
        .is_empty());
        assert_eq!(
            sorted(collect_indices_in_rect(
                Rect::from_min_max(egui::pos2(0.0, 31.0), egui::pos2(100.0, 69.0)),
                metrics,
            )),
            vec![2, 7]
        );
    }

    #[test]
    fn grouped_grid_only_maps_intersected_rows_and_preserves_logical_indices() {
        let sections: Arc<[GroupedRectangleSection]> = vec![GroupedRectangleSection {
            item_indices: (100..10_100).collect::<Vec<_>>().into(),
            origin: egui::pos2(8.0, 38.0),
        }]
        .into();
        let metrics = RectangleSelectionMetrics::Grouped(GroupedRectangleMetrics {
            view: RectangleSelectionView::Grid,
            projection_identity: GroupedProjectionIdentity::new(sections.clone(), 20_000),
            sections,
            layout: GroupedRectangleLayout::Grid {
                cols: 4,
                item_w: 40.0,
                item_h: 50.0,
                column_step: 48.0,
                row_step: 58.0,
            },
            item_count: 20_000,
            content_width: 200.0,
            content_height: 150_000.0,
        });

        assert_eq!(
            sorted(collect_indices_in_rect(
                Rect::from_min_max(egui::pos2(55.0, 97.0), egui::pos2(95.0, 145.0)),
                metrics,
            )),
            vec![105]
        );
    }

    #[test]
    fn grouped_column_list_skips_header_and_maps_top_to_bottom() {
        let sections: Arc<[GroupedRectangleSection]> = vec![GroupedRectangleSection {
            item_indices: vec![9, 4, 7, 2, 8].into(),
            origin: egui::pos2(280.0, 28.0),
        }]
        .into();
        let metrics = RectangleSelectionMetrics::Grouped(GroupedRectangleMetrics {
            view: RectangleSelectionView::ColumnList,
            projection_identity: GroupedProjectionIdentity::new(sections.clone(), 10),
            sections,
            layout: GroupedRectangleLayout::ColumnList {
                rows_per_column: 3,
                column_width: 280.0,
                row_height: 24.0,
            },
            item_count: 10,
            content_width: 840.0,
            content_height: 100.0,
        });

        assert!(collect_indices_in_rect(
            Rect::from_min_max(egui::pos2(280.0, 0.0), egui::pos2(560.0, 27.0)),
            metrics.clone(),
        )
        .is_empty());
        assert_eq!(
            sorted(collect_indices_in_rect(
                Rect::from_min_max(egui::pos2(560.0, 28.0), egui::pos2(840.0, 76.0)),
                metrics,
            )),
            vec![2, 8]
        );
    }

    #[test]
    fn miller_rectangle_state_keeps_its_column_identity() {
        let source = RectangleSelectionSource::MillerAncestor {
            directory: PathBuf::from(r"C:\A"),
        };
        let state = RectangleSelectionState::new_for_source(
            RectangleSelectionView::List,
            source.clone(),
            egui::pos2(0.0, 0.0),
            FxHashSet::default(),
            FxHashSet::default(),
            RectangleSelectionModifiers::default(),
            42,
        );

        assert_eq!(state.source, source);
    }
}
