use eframe::egui::{self, Color32, Pos2, Rect, Stroke};
use std::path::PathBuf;

use crate::ui::cache::FxHashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RectangleSelectionView {
    Grid,
    List,
    ColumnList,
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

#[derive(Clone, Copy, Debug)]
pub enum RectangleSelectionMetrics {
    Grid(GridRectangleMetrics),
    List(ListRectangleMetrics),
    ColumnList(ColumnListRectangleMetrics),
}

impl RectangleSelectionMetrics {
    pub fn view(self) -> RectangleSelectionView {
        match self {
            Self::Grid(_) => RectangleSelectionView::Grid,
            Self::List(_) => RectangleSelectionView::List,
            Self::ColumnList(_) => RectangleSelectionView::ColumnList,
        }
    }

    pub fn content_width(self) -> f32 {
        match self {
            Self::Grid(metrics) => metrics.content_width,
            Self::List(metrics) => metrics.content_width,
            Self::ColumnList(metrics) => metrics.content_width,
        }
    }

    pub fn content_height(self) -> f32 {
        match self {
            Self::Grid(metrics) => metrics.content_height,
            Self::List(metrics) => metrics.content_height,
            Self::ColumnList(metrics) => metrics.content_height,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RectangleSelectionState {
    pub view: RectangleSelectionView,
    pub anchor_content: Pos2,
    pub current_content: Pos2,
    pub base_selection: FxHashSet<PathBuf>,
    pub base_preview_indices: FxHashSet<usize>,
    pub hit_indices: FxHashSet<usize>,
    pub preview_indices: FxHashSet<usize>,
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
        Self {
            view,
            anchor_content,
            current_content: anchor_content,
            base_selection,
            base_preview_indices,
            hit_indices: FxHashSet::default(),
            preview_indices: FxHashSet::default(),
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
        let metrics = self.metrics?;
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
    }
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
}
