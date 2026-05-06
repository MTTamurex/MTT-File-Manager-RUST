use eframe::egui::{self, Color32, Pos2, Rect, Stroke};
use std::path::PathBuf;

use crate::domain::file_entry::FileEntry;
use crate::ui::cache::FxHashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RectangleSelectionView {
    Grid,
    List,
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
pub enum RectangleSelectionMetrics {
    Grid(GridRectangleMetrics),
    List(ListRectangleMetrics),
}

impl RectangleSelectionMetrics {
    pub fn view(self) -> RectangleSelectionView {
        match self {
            Self::Grid(_) => RectangleSelectionView::Grid,
            Self::List(_) => RectangleSelectionView::List,
        }
    }

    pub fn content_width(self) -> f32 {
        match self {
            Self::Grid(metrics) => metrics.content_width,
            Self::List(metrics) => metrics.content_width,
        }
    }

    pub fn content_height(self) -> f32 {
        match self {
            Self::Grid(metrics) => metrics.content_height,
            Self::List(metrics) => metrics.content_height,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RectangleSelectionState {
    pub view: RectangleSelectionView,
    pub anchor_content: Pos2,
    pub current_content: Pos2,
    pub base_selection: FxHashSet<PathBuf>,
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
        modifiers: RectangleSelectionModifiers,
        generation: usize,
    ) -> Self {
        Self {
            view,
            anchor_content,
            current_content: anchor_content,
            base_selection,
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
    pub metrics: Option<RectangleSelectionMetrics>,
    pub start_screen_pos: Option<Pos2>,
}

impl RectangleSelectionFrame {
    pub fn begin(
        &mut self,
        viewport_rect: Rect,
        current_scroll_y: f32,
        max_scroll_y: f32,
        metrics: Option<RectangleSelectionMetrics>,
    ) {
        self.viewport_rect = Some(viewport_rect);
        self.current_scroll_y = current_scroll_y;
        self.max_scroll_y = max_scroll_y;
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
        let x = (screen_pos.x - viewport.left()).clamp(0.0, metrics.content_width().max(0.0));
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
    }
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
    a.min.x <= b.max.x && b.min.x <= a.max.x && a.min.y <= b.max.y && b.min.y <= a.max.y
}

pub fn paint_overlay(
    ui: &egui::Ui,
    state: &RectangleSelectionState,
    viewport_rect: Rect,
    current_scroll_y: f32,
) {
    let content_rect = state.content_rect();
    let screen_rect = Rect::from_min_max(
        egui::pos2(
            viewport_rect.left() + content_rect.left(),
            viewport_rect.top() + content_rect.top() - current_scroll_y,
        ),
        egui::pos2(
            viewport_rect.left() + content_rect.right(),
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

pub fn grid_item_content_contains(
    item: &FileEntry,
    rect: Rect,
    thumbnail_size: f32,
    point: Pos2,
) -> bool {
    if item.drive_info.is_some() {
        return grid_drive_content_contains(rect, thumbnail_size, point);
    }
    if item.is_dir && !item.is_archive() {
        return grid_folder_content_contains(rect, thumbnail_size, point);
    }
    grid_file_content_contains(rect, thumbnail_size, point)
}

fn grid_file_content_contains(rect: Rect, thumbnail_size: f32, point: Pos2) -> bool {
    let available_h = rect.height();
    let available_w = rect.width();
    let thumb_size = (thumbnail_size - 6.0).min(available_w - 4.0).max(1.0);
    let text_height = 18.0;
    let content_h = thumb_size + text_height;
    let vertical_margin = ((available_h - content_h) / 2.0).max(2.0);
    let x_offset = (available_w - thumb_size) / 2.0;
    let thumb_rect = Rect::from_min_size(
        rect.min + egui::vec2(x_offset.max(0.0), vertical_margin),
        egui::vec2(thumb_size, thumb_size),
    );
    let text_rect = Rect::from_min_size(
        egui::pos2(rect.left(), thumb_rect.bottom() + 4.0),
        egui::vec2(rect.width(), 20.0),
    );
    thumb_rect.expand(2.0).contains(point) || text_rect.expand(2.0).contains(point)
}

fn grid_folder_content_contains(rect: Rect, thumbnail_size: f32, point: Pos2) -> bool {
    let available_h = rect.height();
    let folder_w = thumbnail_size * 0.85;
    let folder_h = folder_w * 0.85;
    let text_height = 18.0;
    let content_h = folder_h + text_height;
    let vertical_margin = ((available_h - content_h) / 2.0).max(2.0);
    let x_offset = (rect.width() - folder_w) / 2.0;
    let folder_rect = Rect::from_min_size(
        rect.min + egui::vec2(x_offset.max(0.0), vertical_margin),
        egui::vec2(folder_w, folder_h),
    );
    let text_rect = Rect::from_min_size(
        egui::pos2(rect.left(), folder_rect.bottom() + 6.0),
        egui::vec2(rect.width(), 20.0),
    );
    folder_rect.expand(2.0).contains(point) || text_rect.expand(2.0).contains(point)
}

fn grid_drive_content_contains(rect: Rect, thumbnail_size: f32, point: Pos2) -> bool {
    let available_h = rect.height();
    let available_w = rect.width();
    let icon_size = (thumbnail_size * 0.4).min(available_w * 0.5);
    let progress_w = (available_w * 0.8).min(150.0);
    let text_height = 36.0;
    let content_h = icon_size + 12.0 + 8.0 + text_height;
    let vertical_margin = ((available_h - content_h) / 2.0).max(2.0);

    let mut current_y = rect.top() + vertical_margin;
    let icon_rect = Rect::from_center_size(
        egui::pos2(rect.center().x, current_y + icon_size / 2.0),
        egui::vec2(icon_size, icon_size),
    );
    current_y += icon_size + 8.0;

    let bar_rect = Rect::from_center_size(
        egui::pos2(rect.center().x, current_y + 6.0),
        egui::vec2(progress_w, 12.0),
    );
    current_y += 18.0;

    let name_rect = Rect::from_center_size(
        egui::pos2(rect.center().x, current_y + 9.0),
        egui::vec2(progress_w, 18.0),
    );
    current_y += 18.0;

    let details_rect = Rect::from_center_size(
        egui::pos2(rect.center().x, current_y + 9.0),
        egui::vec2(progress_w, 18.0),
    );

    [icon_rect, bar_rect, name_rect, details_rect]
        .into_iter()
        .any(|rect| rect.expand(2.0).contains(point))
}
