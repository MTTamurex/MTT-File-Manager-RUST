pub const ROW_HEIGHT: f32 = 24.0;
pub const COLUMN_WIDTH: f32 = 280.0;
pub const SCROLLBAR_HEIGHT: f32 = 14.0;

pub(super) fn header_height(ui: &eframe::egui::Ui) -> f32 {
    crate::ui::fonts::layout::row_height(ui, 28.0, 13.0, 8.0)
}

pub fn column_list_rows(
    ui: &eframe::egui::Ui,
    item_count: usize,
    available_width: f32,
    available_height: f32,
) -> usize {
    calculate_layout(
        item_count,
        available_width,
        available_height,
        row_height(ui),
    )
    .rows_per_column
}

pub fn column_list_grouped_rows(
    ui: &eframe::egui::Ui,
    local_count: usize,
    network_count: usize,
    available_width: f32,
    available_height: f32,
) -> usize {
    column_list_grouped_rows_for_counts(
        ui,
        &[local_count, network_count],
        available_width,
        available_height,
    )
}

pub fn column_list_grouped_rows_for_counts(
    ui: &eframe::egui::Ui,
    group_counts: &[usize],
    available_width: f32,
    available_height: f32,
) -> usize {
    calculate_grouped_layout(
        group_counts,
        available_width,
        (available_height - header_height(ui)).max(0.0),
        row_height(ui),
    )
    .rows_per_column
}

pub fn column_list_visible_columns(available_width: f32) -> usize {
    (available_width / COLUMN_WIDTH).floor().max(1.0) as usize
}

pub(super) fn row_height(ui: &eframe::egui::Ui) -> f32 {
    crate::ui::fonts::layout::row_height(ui, ROW_HEIGHT, 12.0, 6.0)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColumnListLayout {
    pub rows_per_column: usize,
    pub column_count: usize,
    pub content_width: f32,
    pub viewport_height: f32,
    pub has_horizontal_scrollbar: bool,
}

pub fn calculate_layout(
    item_count: usize,
    available_width: f32,
    available_height: f32,
    row_height: f32,
) -> ColumnListLayout {
    calculate_grouped_layout(&[item_count], available_width, available_height, row_height)
}

pub fn calculate_grouped_layout(
    group_counts: &[usize],
    available_width: f32,
    available_height: f32,
    row_height: f32,
) -> ColumnListLayout {
    let initial_rows = (available_height / row_height).floor().max(1.0) as usize;
    let initial_columns = group_counts
        .iter()
        .map(|count| count.div_ceil(initial_rows))
        .sum::<usize>();
    let has_horizontal_scrollbar = initial_columns as f32 * COLUMN_WIDTH > available_width;
    let viewport_height = if has_horizontal_scrollbar {
        (available_height - SCROLLBAR_HEIGHT).max(0.0)
    } else {
        available_height.max(0.0)
    };
    let rows_per_column = (viewport_height / row_height).floor().max(1.0) as usize;
    let column_count = group_counts
        .iter()
        .map(|count| count.div_ceil(rows_per_column))
        .sum::<usize>();

    ColumnListLayout {
        rows_per_column,
        column_count,
        content_width: column_count as f32 * COLUMN_WIDTH,
        viewport_height,
        has_horizontal_scrollbar: column_count as f32 * COLUMN_WIDTH > available_width,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_each_column_top_to_bottom() {
        let layout = calculate_layout(10, 300.0, 86.0, ROW_HEIGHT);
        assert_eq!(layout.rows_per_column, 3);
        assert_eq!(layout.column_count, 4);
        assert_eq!(7 / layout.rows_per_column, 2);
        assert_eq!(7 % layout.rows_per_column, 1);
    }

    #[test]
    fn omits_scrollbar_when_all_columns_fit() {
        let layout = calculate_layout(4, 500.0, 100.0, ROW_HEIGHT);
        assert!(!layout.has_horizontal_scrollbar);
        assert_eq!(layout.column_count, 1);
    }

    #[test]
    fn grouped_layout_starts_each_group_in_a_new_column() {
        let layout = calculate_grouped_layout(&[2, 2], 900.0, 240.0, ROW_HEIGHT);
        assert_eq!(layout.rows_per_column, 10);
        assert_eq!(layout.column_count, 2);
    }

    #[test]
    fn taller_rows_change_column_geometry_consistently() {
        let normal = calculate_layout(30, 300.0, 240.0, ROW_HEIGHT);
        let larger = calculate_layout(30, 300.0, 240.0, 36.0);
        assert!(larger.rows_per_column < normal.rows_per_column);
        assert!(larger.column_count > normal.column_count);
        assert!(larger.rows_per_column as f32 * 36.0 <= larger.viewport_height);
    }
}
