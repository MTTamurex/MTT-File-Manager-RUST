//! Single source of geometry and identity for headers, cells and hit testing.
use super::ListViewContext;
use crate::domain::file_entry::SortMode;
use crate::domain::list_columns::{ColumnId, ColumnScope};
use rust_i18n::t;

#[derive(Clone, Copy, Debug)]
pub(crate) struct ColumnCell {
    pub id: ColumnId,
    pub offset: f32,
    pub width: f32,
}

pub(super) fn scope(ctx: &ListViewContext) -> ColumnScope {
    ColumnScope::from_view(ctx.is_computer_view, ctx.is_recycle_bin_view)
}

pub(super) fn cells(ctx: &ListViewContext) -> Vec<ColumnCell> {
    project(
        ctx.columns.preferences.layout(scope(ctx)),
        ctx.is_onedrive_folder,
        |id| width(ctx, id),
    )
}

fn project(
    layout: &crate::domain::list_columns::ColumnLayout,
    cloud: bool,
    width: impl Fn(ColumnId) -> f32,
) -> Vec<ColumnCell> {
    let mut offset = 0.0;
    layout
        .columns
        .iter()
        .filter(|column| column.visible && (column.id != ColumnId::Status || cloud))
        .map(|column| {
            let width = width(column.id).max(super::fit_panel::min_width(column.id));
            let cell = ColumnCell {
                id: column.id,
                offset,
                width,
            };
            offset += width;
            cell
        })
        .collect()
}

pub(super) fn width(ctx: &ListViewContext, id: ColumnId) -> f32 {
    if let Some(width) = ctx
        .width_overrides
        .as_ref()
        .and_then(|widths| widths.iter().find(|(column, _)| *column == id))
    {
        return width.1;
    }
    match id {
        ColumnId::Name => *ctx.col_name_width,
        ColumnId::Date if !ctx.is_computer_view => *ctx.col_date_width,
        ColumnId::TotalSpace if ctx.is_computer_view => *ctx.col_date_width,
        ColumnId::Type if !ctx.is_computer_view => *ctx.col_type_width,
        ColumnId::Size if !ctx.is_computer_view => *ctx.col_size_width,
        ColumnId::FreeSpace if ctx.is_computer_view => *ctx.col_size_width,
        ColumnId::Status => *ctx.col_status_width,
        _ => ctx
            .columns
            .preferences
            .layout(scope(ctx))
            .columns
            .iter()
            .find(|column| column.id == id)
            .map(|column| column.width)
            .unwrap_or(160.0),
    }
}

pub(super) fn set_width(ctx: &mut ListViewContext, id: ColumnId, width: f32) {
    if let Some(column) = ctx
        .width_overrides
        .as_mut()
        .and_then(|widths| widths.iter_mut().find(|(column, _)| *column == id))
    {
        column.1 = width;
    }
    if ctx.folder_width_location.is_some() {
        ctx.manual_width_changes.push((id, width));
        return;
    }
    if set_legacy_width(ctx, id, width) {
        return;
    }
    let scope = scope(ctx);
    if let Some(column) = ctx
        .columns
        .preferences
        .layout_mut(scope)
        .columns
        .iter_mut()
        .find(|column| column.id == id)
    {
        column.width = width;
    }
}

fn set_legacy_width(ctx: &mut ListViewContext, id: ColumnId, width: f32) -> bool {
    let target = match id {
        ColumnId::Name => &mut *ctx.col_name_width,
        ColumnId::Date if !ctx.is_computer_view => &mut *ctx.col_date_width,
        ColumnId::TotalSpace if ctx.is_computer_view => &mut *ctx.col_date_width,
        ColumnId::Type if !ctx.is_computer_view => &mut *ctx.col_type_width,
        ColumnId::Size if !ctx.is_computer_view => &mut *ctx.col_size_width,
        ColumnId::FreeSpace if ctx.is_computer_view => &mut *ctx.col_size_width,
        ColumnId::Status => &mut *ctx.col_status_width,
        _ => return false,
    };
    *target = width;
    true
}

/// Automatic metadata widths belong to a physical pane, not the global manual-width preferences.
pub(super) fn set_fitted_widths(ctx: &mut ListViewContext, widths: &[(ColumnId, f32)]) {
    ctx.width_overrides = Some(widths.to_vec());
    for &(id, width) in widths {
        set_legacy_width(ctx, id, width);
    }
}

pub(super) fn label(ctx: &ListViewContext, id: ColumnId) -> String {
    match id {
        ColumnId::Name => t!("list_view.name").to_string(),
        ColumnId::Date => t!(if ctx.is_recycle_bin_view {
            "list_view.date_deleted"
        } else {
            "list_view.date_modified"
        })
        .to_string(),
        ColumnId::Type => t!("list_view.type_col").to_string(),
        ColumnId::Size => t!("list_view.size_col").to_string(),
        ColumnId::TotalSpace => t!("list_view.total_space").to_string(),
        ColumnId::FreeSpace => t!("list_view.free_space").to_string(),
        ColumnId::Tags => t!("column_headers.app_tags").to_string(),
        ColumnId::OriginalLocation => t!("column_headers.original_location").to_string(),
        ColumnId::Status => t!("column_headers.status").to_string(),
        ColumnId::Property(key) => ctx
            .columns
            .catalog
            .iter()
            .find(|column| column.id == id)
            .map(|column| column.label.clone())
            .unwrap_or_else(|| format!("{:032x}:{}", key.fmtid, key.pid)),
    }
}

pub(super) fn sort_mode(id: ColumnId) -> Option<SortMode> {
    Some(match id {
        ColumnId::Name => SortMode::Name,
        ColumnId::Date => SortMode::Date,
        ColumnId::Type => SortMode::Type,
        ColumnId::Size => SortMode::Size,
        ColumnId::TotalSpace => SortMode::DriveTotalSpace,
        ColumnId::FreeSpace => SortMode::DriveFreeSpace,
        ColumnId::Status => return None,
        _ => SortMode::Column(id),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::list_columns::ColumnLayout;
    #[test]
    fn geometry_follows_visible_order_including_a_moved_name_column() {
        let mut layout = ColumnLayout::defaults(ColumnScope::Files);
        layout.set_visible(ColumnId::Type, false);
        layout.move_before(ColumnId::Size, Some(ColumnId::Name));
        let cells = project(&layout, false, |_| 160.0);
        assert_eq!(
            cells.iter().map(|cell| cell.id).collect::<Vec<_>>(),
            vec![ColumnId::Size, ColumnId::Name, ColumnId::Date]
        );
        assert_eq!(cells[1].offset, 160.0);
        assert_eq!(
            cells.last().unwrap().offset + cells.last().unwrap().width,
            480.0
        );
    }
    #[test]
    fn cloud_status_is_contextual_without_losing_its_saved_position() {
        let mut layout = ColumnLayout::defaults(ColumnScope::Files);
        layout.move_before(ColumnId::Status, Some(ColumnId::Name));
        assert_eq!(project(&layout, true, |_| 100.0)[0].id, ColumnId::Status);
        assert_eq!(project(&layout, false, |_| 100.0)[0].id, ColumnId::Name);
        assert_eq!(project(&layout, true, |_| 100.0)[0].id, ColumnId::Status);
    }
}
