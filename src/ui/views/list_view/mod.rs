//! Details view: shared column layout, virtualized rows and independent interactions.
pub(crate) mod autofit;
pub(crate) mod autofit_layout;
mod cell_content;
mod column_choices;
mod column_picker;
mod columns;
mod context;
mod header;
mod helpers;
mod horizontal;
pub(crate) mod item_renderer;
pub(crate) mod item_renderer_details;
mod name_cell;
mod scroll;
mod text_layout;
mod virtualization;

use crate::domain::list_columns::ColumnId;
pub(crate) use context::ColumnWidths;
pub use context::{ListViewAction, ListViewContext, ListViewOperations};
use eframe::egui::Ui;
pub(crate) use text_layout::{clear_font_metrics_cache, truncate_text_for_column};

pub fn render_list_view(
    ui: &mut Ui,
    ctx: &mut ListViewContext,
    ops: &mut dyn ListViewOperations,
) -> Option<ListViewAction> {
    if let Some(location) = &ctx.folder_width_location {
        let defaults: Vec<_> = ctx
            .columns
            .preferences
            .layout(columns::scope(ctx))
            .columns
            .iter()
            .map(|column| (column.id, columns::width(ctx, column.id)))
            .collect();
        ctx.width_overrides = Some(
            ctx.columns
                .folder_widths
                .frame(location, &defaults, ui.ctx()),
        );
    }
    crate::ui::components::inline_rename::clear_pending_name_click_outside(
        ui.ctx(),
        ctx.rename_click_scope,
        ctx.selected_item
            .and_then(|index| ctx.items.get(index))
            .map(|item| item.path.as_path()),
    );
    let row_height = crate::ui::fonts::layout::row_height(ui, 24.0, 12.0, 6.0);
    let available = ui.available_width();
    autofit::update(ui, ctx);
    horizontal::reveal_name_for_rename(ctx, available);
    horizontal::prepare(ui, ctx);
    let layout_before = ctx.columns.preferences.layout(columns::scope(ctx)).clone();
    let visible_before: Vec<_> = columns::cells(ctx).iter().map(|cell| cell.id).collect();
    let sort = if ctx.compact {
        None
    } else {
        let action = horizontal::render_header(ui, ctx, available);
        ui.separator();
        action
    };
    if !ctx.compact && layout_before != *ctx.columns.preferences.layout(columns::scope(ctx)) {
        let visible_after: Vec<_> = columns::cells(ctx).iter().map(|cell| cell.id).collect();
        if visible_before.len() != visible_after.len()
            || visible_before.iter().any(|id| !visible_after.contains(id))
        {
            autofit::update(ui, ctx);
        }
        ui.ctx().request_repaint();
    }
    let widths = if ctx.compact {
        ColumnWidths {
            name: (available - 8.0).max(50.0),
            cells: Vec::new(),
            name_offset: 0.0,
        }
    } else {
        let cells = columns::cells(ctx);
        let name_cell = cells.iter().find(|cell| cell.id == ColumnId::Name);
        let name_offset = name_cell.map(|cell| cell.offset).unwrap_or(0.0);
        let name = name_cell.map(|cell| cell.width).unwrap_or(100.0);
        ColumnWidths {
            name,
            cells,
            name_offset,
        }
    };
    let interaction =
        virtualization::render_virtualized_content(ui, ctx, ops, available, row_height, &widths);
    autofit::remember_visible_rows(ui, ctx);
    if let Some(key) = interaction.toggled_group {
        return Some(ListViewAction::ToggleGroup(key));
    }
    if let Some(index) = ctx.rename_requested_item.take() {
        return Some(ListViewAction::StartRename(index));
    }
    if interaction.empty_area_secondary_click {
        return Some(ListViewAction::EmptyAreaSecondaryClick);
    }
    if interaction.empty_area_click {
        return Some(ListViewAction::EmptyAreaClick);
    }
    if let Some(mode) = sort {
        return Some(ListViewAction::SortChange(mode));
    }
    if let Some(index) = interaction.double_clicked_item {
        return Some(ListViewAction::DoubleClick(index));
    }
    if let Some(index) = interaction.secondary_clicked_item {
        return Some(ListViewAction::SecondaryClick(index));
    }
    interaction.clicked_item.map(ListViewAction::Click)
}
