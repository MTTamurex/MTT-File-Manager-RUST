use std::ops::Range;

use eframe::egui;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct PageGeometry {
    pub(super) scale: f32,
    pub(super) size: egui::Vec2,
}

#[derive(Clone, Copy, Debug)]
struct Row {
    top: f32,
    height: f32,
}

#[derive(Debug)]
pub(super) struct VariableRows {
    rows: Vec<Row>,
    total_height: f32,
}

impl VariableRows {
    pub(super) fn new(heights: impl IntoIterator<Item = f32>, gap: f32) -> Self {
        let gap = gap.max(0.0);
        let mut rows = Vec::new();
        let mut top = 0.0;

        for height in heights {
            let height = height.max(0.0);
            rows.push(Row { top, height });
            top += height + gap;
        }

        Self {
            rows,
            total_height: top,
        }
    }

    pub(super) fn total_height(&self) -> f32 {
        self.total_height
    }

    pub(super) fn top(&self, row: usize) -> Option<f32> {
        self.rows.get(row).map(|row| row.top)
    }

    pub(super) fn height(&self, row: usize) -> Option<f32> {
        self.rows.get(row).map(|row| row.height)
    }

    pub(super) fn visible_range(&self, viewport: egui::Rect, overscan: usize) -> Range<usize> {
        let first = self
            .rows
            .partition_point(|row| row.top + row.height <= viewport.min.y);
        let end = self.rows.partition_point(|row| row.top < viewport.max.y);

        first.saturating_sub(overscan)..(end + overscan).min(self.rows.len())
    }

    pub(super) fn centered_scroll_offset(&self, row: usize, viewport_height: f32) -> Option<f32> {
        let row = self.rows.get(row)?;
        Some((row.top + row.height * 0.5 - viewport_height * 0.5).max(0.0))
    }
}

#[derive(Debug)]
pub(super) struct PageRows {
    pages: Vec<PageGeometry>,
    rows: VariableRows,
    row_pages: Vec<Range<usize>>,
    page_rows: Vec<usize>,
    horizontal_gap: f32,
    max_row_width: f32,
}

impl PageRows {
    pub(super) fn new(
        pages: Vec<PageGeometry>,
        columns: usize,
        horizontal_gap: f32,
        vertical_gap: f32,
    ) -> Self {
        debug_assert!((1..=2).contains(&columns));
        let row_pages = (0..pages.len())
            .step_by(columns)
            .map(|start| start..(start + columns).min(pages.len()))
            .collect();

        Self::from_row_pages(pages, row_pages, horizontal_gap, vertical_gap)
    }

    pub(super) fn with_cover_page(
        pages: Vec<PageGeometry>,
        horizontal_gap: f32,
        vertical_gap: f32,
    ) -> Self {
        let mut row_pages = Vec::with_capacity(pages.len().div_ceil(2) + 1);
        if !pages.is_empty() {
            row_pages.push(0..1);
        }

        let mut start = 1;
        while start < pages.len() {
            let end = (start + 2).min(pages.len());
            row_pages.push(start..end);
            start = end;
        }

        Self::from_row_pages(pages, row_pages, horizontal_gap, vertical_gap)
    }

    fn from_row_pages(
        pages: Vec<PageGeometry>,
        row_pages: Vec<Range<usize>>,
        horizontal_gap: f32,
        vertical_gap: f32,
    ) -> Self {
        let horizontal_gap = horizontal_gap.max(0.0);
        let mut heights = Vec::with_capacity(row_pages.len());
        let mut max_row_width = 0.0_f32;
        let mut page_rows = vec![0; pages.len()];

        for (row_idx, row_range) in row_pages.iter().enumerate() {
            let row = &pages[row_range.clone()];
            for page_row in &mut page_rows[row_range.clone()] {
                *page_row = row_idx;
            }

            let height = row.iter().map(|page| page.size.y).fold(0.0_f32, f32::max);
            let width = row.iter().map(|page| page.size.x).sum::<f32>()
                + horizontal_gap * row.len().saturating_sub(1) as f32;
            heights.push(height);
            max_row_width = max_row_width.max(width);
        }

        Self {
            pages,
            rows: VariableRows::new(heights, vertical_gap),
            row_pages,
            page_rows,
            horizontal_gap,
            max_row_width,
        }
    }

    pub(super) fn total_height(&self) -> f32 {
        self.rows.total_height()
    }

    pub(super) fn content_width(&self, viewport_width: f32) -> f32 {
        viewport_width.max(self.max_row_width)
    }

    pub(super) fn visible_rows(&self, viewport: egui::Rect, overscan: usize) -> Range<usize> {
        self.rows.visible_range(viewport, overscan)
    }

    pub(super) fn pages_in_row(&self, row: usize) -> Range<usize> {
        self.row_pages
            .get(row)
            .cloned()
            .unwrap_or(self.pages.len()..self.pages.len())
    }

    pub(super) fn page(&self, page: usize) -> Option<PageGeometry> {
        self.pages.get(page).copied()
    }

    pub(super) fn page_top(&self, page: usize) -> Option<f32> {
        let geometry = self.pages.get(page)?;
        let row = *self.page_rows.get(page)?;
        let row_top = self.rows.top(row)?;
        let row_height = self.rows.height(row)?;
        Some(row_top + (row_height - geometry.size.y) * 0.5)
    }

    pub(super) fn page_rect(&self, page: usize, content_width: f32) -> Option<egui::Rect> {
        let geometry = *self.pages.get(page)?;
        let row = *self.page_rows.get(page)?;
        let row_range = self.row_pages.get(row)?;
        let row_pages = &self.pages[row_range.clone()];
        let row_width = row_pages.iter().map(|page| page.size.x).sum::<f32>()
            + self.horizontal_gap * row_pages.len().saturating_sub(1) as f32;
        let preceding_width = self.pages[row_range.start..page]
            .iter()
            .map(|page| page.size.x)
            .sum::<f32>();
        let x = ((content_width - row_width) * 0.5).max(0.0)
            + preceding_width
            + self.horizontal_gap * (page - row_range.start) as f32;

        Some(egui::Rect::from_min_size(
            egui::pos2(x, self.page_top(page)?),
            geometry.size,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(width: f32, height: f32) -> PageGeometry {
        PageGeometry {
            scale: 1.0,
            size: egui::vec2(width, height),
        }
    }

    #[test]
    fn variable_rows_find_only_visible_rows() {
        let rows = VariableRows::new([100.0, 200.0, 50.0], 10.0);
        let viewport = egui::Rect::from_min_max(egui::pos2(0.0, 115.0), egui::pos2(100.0, 150.0));

        assert_eq!(rows.top(1), Some(110.0));
        assert_eq!(rows.total_height(), 380.0);
        assert_eq!(rows.visible_range(viewport, 0), 1..2);
    }

    #[test]
    fn two_page_rows_use_the_tallest_page() {
        let rows = PageRows::new(
            vec![page(100.0, 200.0), page(80.0, 100.0), page(120.0, 90.0)],
            2,
            12.0,
            8.0,
        );

        assert_eq!(rows.page_top(1), Some(50.0));
        assert_eq!(rows.page_top(2), Some(208.0));
        assert_eq!(rows.pages_in_row(1), 2..3);
    }

    #[test]
    fn cover_page_rows_start_with_a_single_page_then_pair_the_rest() {
        let rows = PageRows::with_cover_page(
            vec![
                page(100.0, 200.0),
                page(100.0, 100.0),
                page(100.0, 150.0),
                page(100.0, 120.0),
                page(100.0, 160.0),
                page(100.0, 110.0),
            ],
            12.0,
            8.0,
        );

        assert_eq!(rows.pages_in_row(0), 0..1);
        assert_eq!(rows.pages_in_row(1), 1..3);
        assert_eq!(rows.pages_in_row(2), 3..5);
        assert_eq!(rows.pages_in_row(3), 5..6);
        assert_eq!(rows.page_top(0), Some(0.0));
        assert_eq!(rows.page_top(1), Some(233.0));
        assert_eq!(rows.page_top(3), Some(386.0));
        assert_eq!(rows.page_top(5), Some(534.0));

        let cover_rect = rows.page_rect(0, rows.content_width(0.0)).unwrap();
        assert_eq!(cover_rect.min.x, 56.0);
    }

    #[test]
    fn visible_work_does_not_grow_with_page_count() {
        for page_count in [500, 1_000, 2_000] {
            let rows = PageRows::new(vec![page(600.0, 800.0); page_count], 1, 12.0, 8.0);
            let viewport = egui::Rect::from_min_max(
                egui::pos2(0.0, 500_000.0),
                egui::pos2(1_000.0, 500_800.0),
            );
            assert!(rows.visible_rows(viewport, 1).len() <= 4);
        }
    }

    #[test]
    fn direct_scroll_offset_centers_a_row() {
        let rows = VariableRows::new([100.0, 200.0], 10.0);
        assert_eq!(rows.centered_scroll_offset(1, 100.0), Some(160.0));
    }
}
