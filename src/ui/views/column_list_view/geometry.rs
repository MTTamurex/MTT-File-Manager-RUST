pub const ROW_HEIGHT: f32 = 24.0;
pub const COLUMN_WIDTH: f32 = 280.0;
pub const SCROLLBAR_HEIGHT: f32 = 14.0;

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
) -> ColumnListLayout {
    calculate_grouped_layout(&[item_count], available_width, available_height)
}

pub fn calculate_grouped_layout(
    group_counts: &[usize],
    available_width: f32,
    available_height: f32,
) -> ColumnListLayout {
    let initial_rows = (available_height / ROW_HEIGHT).floor().max(1.0) as usize;
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
    let rows_per_column = (viewport_height / ROW_HEIGHT).floor().max(1.0) as usize;
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
        let layout = calculate_layout(10, 300.0, 86.0);
        assert_eq!(layout.rows_per_column, 3);
        assert_eq!(layout.column_count, 4);
        assert_eq!(7 / layout.rows_per_column, 2);
        assert_eq!(7 % layout.rows_per_column, 1);
    }

    #[test]
    fn omits_scrollbar_when_all_columns_fit() {
        let layout = calculate_layout(4, 500.0, 100.0);
        assert!(!layout.has_horizontal_scrollbar);
        assert_eq!(layout.column_count, 1);
    }

    #[test]
    fn grouped_layout_starts_each_group_in_a_new_column() {
        let layout = calculate_grouped_layout(&[2, 2], 900.0, 240.0);
        assert_eq!(layout.rows_per_column, 10);
        assert_eq!(layout.column_count, 2);
    }
}
