//! Cell values, painting and content hit testing use the same column rectangles.
use super::{truncate_text_for_column, ColumnWidths, ListViewContext};
use crate::domain::file_entry::FileEntry;
use crate::domain::list_columns::ColumnId;
use crate::infrastructure::windows::{format_date, format_size};
use eframe::egui::{self, Color32, FontId, Pos2, Rect, Ui};

/// The same cache-only size is used by painting and automatic-width invalidation.
pub(super) fn cached_size(item: &FileEntry, ctx: &ListViewContext) -> Option<u64> {
    if item.is_dir && !item.is_archive() {
        ctx.folder_size_display_cache
            .peek(&item.path)
            .map(|confirmed| confirmed.size)
            .or_else(|| ctx.folder_size_progress.get(&item.path).copied())
            .or_else(|| ctx.folder_size_cache.peek(&item.path).copied())
            .or_else(|| ctx.folder_sort_sizes.get(&item.path).copied())
    } else {
        Some(
            crate::app::live_file_size::cached_live_file_size(
                &item.path,
                item.modified,
                ctx.live_file_size_cache,
            )
            .unwrap_or(item.size),
        )
    }
}

pub(super) fn text(
    ui: &Ui,
    item: &FileEntry,
    ctx: &mut ListViewContext,
    id: ColumnId,
    enqueue: bool,
) -> String {
    match id {
        ColumnId::Name => {
            crate::ui::components::item_slot::display_name_for_item(item).into_owned()
        }
        ColumnId::Type => super::helpers::get_file_type_string(item),
        ColumnId::Date if ctx.is_recycle_bin_view => {
            if item.modified > 0 {
                format_date(item.modified)
            } else {
                item.deletion_date()
                    .map(crate::infrastructure::windows::format_metadata_date)
                    .unwrap_or_default()
            }
        }
        ColumnId::Date => format_date(item.modified),
        ColumnId::TotalSpace => item
            .drive_info
            .as_ref()
            .map(|drive| format_size(drive.total_space))
            .unwrap_or_default(),
        ColumnId::FreeSpace => item
            .drive_info
            .as_ref()
            .map(|drive| format_size(drive.free_space))
            .unwrap_or_default(),
        ColumnId::OriginalLocation => item
            .recycle_original_path()
            .and_then(|path| path.parent())
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default(),
        ColumnId::Tags => {
            crate::domain::file_tag::tag_ids_for_path(ctx.tag_assignments, &item.path)
                .into_iter()
                .flatten()
                .filter_map(|id| ctx.tag_definitions.get(id))
                .map(|tag| tag.name.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        }
        ColumnId::Property(property) => {
            if enqueue {
                ctx.columns.value(ui.ctx(), item, ctx.generation, property)
            } else {
                ctx.columns
                    .cached_value(item, ctx.generation, property)
                    .unwrap_or_default()
            }
        }
        ColumnId::Status => String::new(),
        ColumnId::Size if item.is_dir && !item.is_archive() => {
            // A display-only old value must never suppress a fresh request.
            if enqueue
                && !ctx.folder_size_cache.contains(&item.path)
                && !ctx.folder_sort_sizes.contains_key(&item.path)
                && !ctx.folder_size_batch_loading.contains(&item.path)
            {
                ctx.folder_size_requests.push(item.path.clone());
            }
            if let Some(size) = cached_size(item, ctx) {
                format_size(size)
            } else {
                String::new()
            }
        }
        ColumnId::Size => {
            let size = if enqueue {
                crate::app::live_file_size::resolve_cached_or_enqueue_live_file_size(
                    &item.path,
                    item.modified,
                    item.size,
                    ctx.live_file_size_cache,
                    ctx.live_file_size_loading,
                    ctx.live_file_size_req_sender,
                )
            } else {
                cached_size(item, ctx).unwrap_or(item.size)
            };
            format_size(size)
        }
    }
}

pub(super) fn render(
    ui: &mut Ui,
    item: &FileEntry,
    row: Rect,
    widths: &ColumnWidths,
    ctx: &mut ListViewContext,
    color: Color32,
) {
    ctx.columns.request_values(
        ui.ctx(),
        item,
        ctx.generation,
        widths.cells.iter().filter_map(|cell| match cell.id {
            ColumnId::Property(property) => Some(property),
            _ => None,
        }),
    );
    for cell in &widths.cells {
        if cell.id == ColumnId::Name {
            continue;
        }
        let rect = Rect::from_min_size(
            row.min + egui::vec2(cell.offset, 0.0),
            egui::vec2(cell.width, row.height()),
        );
        if !ui.is_rect_visible(rect) {
            continue;
        }
        let clip = ui.clip_rect();
        ui.set_clip_rect(clip.intersect(rect));
        if cell.id == ColumnId::Status {
            super::helpers::render_status_badge(
                ui,
                rect.min + egui::vec2(8.0, 4.0),
                item.sync_status,
            );
        } else {
            let value = text(ui, item, ctx, cell.id, true);
            let font = FontId::proportional(12.0);
            let display = truncate_text_for_column(&value, cell.width - 8.0, &font, ui);
            ui.painter().text(
                rect.min + egui::vec2(0.0, 5.0),
                egui::Align2::LEFT_TOP,
                display,
                font,
                color,
            );
        }
        ui.set_clip_rect(clip);
    }
}

pub(super) fn contains(
    ui: &Ui,
    item: &FileEntry,
    ctx: &mut ListViewContext,
    row: Rect,
    widths: &ColumnWidths,
    point: Pos2,
) -> bool {
    let name_start = row.min + egui::vec2(widths.name_offset, 0.0);
    if Rect::from_min_size(name_start + egui::vec2(2.0, 2.0), egui::vec2(20.0, 20.0))
        .contains(point)
    {
        return true;
    }
    let tag_offset = if super::name_cell::first_tag_color(item, ctx).is_some() {
        12.0
    } else {
        0.0
    };
    let font = FontId::proportional(12.0);
    let hit_text = |value: &str, origin: Pos2, width: f32| {
        if value.is_empty() || width <= 0.0 {
            return false;
        }
        let measured = ui
            .painter()
            .layout_no_wrap(value.to_owned(), font.clone(), Color32::WHITE)
            .rect
            .width();
        Rect::from_min_size(
            origin,
            egui::vec2(measured.min(width), (row.height() - 6.0).max(12.0)),
        )
        .expand(3.0)
        .contains(point)
    };
    if tag_offset > 0.0
        && Rect::from_center_size(
            Pos2::new(name_start.x + 27.0, row.center().y),
            egui::vec2(13.0, 13.0),
        )
        .contains(point)
    {
        return true;
    }
    if hit_text(
        &crate::ui::components::item_slot::display_name_for_item(item),
        name_start + egui::vec2(24.0 + tag_offset, 5.0),
        widths.name - 30.0 - tag_offset,
    ) {
        return true;
    }
    if ctx.compact {
        return false;
    }
    for cell in &widths.cells {
        if cell.id == ColumnId::Name {
            continue;
        }
        let origin = row.min + egui::vec2(cell.offset, 5.0);
        if point.x < origin.x || point.x > origin.x + cell.width {
            continue;
        }
        if cell.id == ColumnId::Status {
            return item.sync_status != crate::domain::file_entry::SyncStatus::None
                && Rect::from_min_size(origin, egui::vec2(28.0, 16.0)).contains(point);
        }
        return hit_text(
            &text(ui, item, ctx, cell.id, false),
            origin,
            cell.width - 8.0,
        );
    }
    false
}
