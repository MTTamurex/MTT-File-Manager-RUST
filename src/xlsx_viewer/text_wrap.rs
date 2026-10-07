use std::ops::Range;

use betteroffice_xlsx::{Align, CellRef, DrawCmd, Rect, SheetId, Workbook};
use xlsx_model::VAlign;
use xlsx_render::{display_text, DisplayList};

#[derive(Clone, Copy)]
enum VerticalAlignment {
    Top,
    Center,
    Bottom,
}

struct VisibleCellText {
    text: String,
    wrap_text: bool,
    vertical: VerticalAlignment,
    cell_clip: Option<Rect>,
    blocks_left: bool,
    blocks_right: bool,
}

pub(super) fn wrap_display_list(
    workbook: &Workbook,
    sheet_id: SheetId,
    display_list: &mut DisplayList,
) {
    let visible_cells = visible_cell_texts(workbook, sheet_id, display_list);
    rewrite_text_commands(&mut display_list.commands, &visible_cells);
}

fn visible_cell_texts(
    workbook: &Workbook,
    sheet_id: SheetId,
    display_list: &DisplayList,
) -> Vec<VisibleCellText> {
    let model = workbook.model();
    let Some(sheet) = model.sheet(sheet_id) else {
        return Vec::new();
    };
    let row_indices = visible_indices(
        display_list.grid.start_row,
        display_list.grid.row_indices.as_deref(),
        display_list.grid.row_offsets.len(),
    );
    let col_indices = visible_indices(
        display_list.grid.start_col,
        display_list.grid.col_indices.as_deref(),
        display_list.grid.col_offsets.len(),
    );
    let row_ranges = visible_ranges(&row_indices);
    let col_ranges = visible_ranges(&col_indices);
    let mut cells = Vec::new();
    for row_range in row_ranges {
        for col_range in &col_ranges {
            cells.extend(sheet.iter_cells_in_rect(row_range.clone(), col_range.clone()));
        }
    }
    cells.sort_unstable_by_key(|(cell, _)| (cell.row, cell.col));
    cells.dedup_by_key(|(cell, _)| *cell);

    cells
        .into_iter()
        .filter_map(|(cell_ref, cell)| {
            let merged = sheet.merges.iter().find(|merge| merge.contains(cell_ref));
            if merged.is_some_and(|merge| merge.start != cell_ref) {
                return None;
            }
            let text = display_text(&model.styles, model.date_system, cell);
            if text.is_empty() {
                return None;
            }
            let alignment = cell
                .style
                .and_then(|style| model.styles.alignment_for(style));
            let vertical = match alignment.and_then(|alignment| alignment.v) {
                Some(VAlign::Top) => VerticalAlignment::Top,
                Some(VAlign::Center | VAlign::Justify | VAlign::Distributed) => {
                    VerticalAlignment::Center
                }
                Some(VAlign::Bottom) | None => VerticalAlignment::Bottom,
            };
            let cell_clip = if merged.is_none() {
                cell_bounds(&display_list.grid, &row_indices, &col_indices, cell_ref)
            } else {
                None
            };
            let blocks_left = cell_ref.col > 0
                && has_blocking_neighbor(sheet, CellRef::new(cell_ref.row, cell_ref.col - 1));
            let blocks_right = has_blocking_neighbor(
                sheet,
                CellRef::new(cell_ref.row, cell_ref.col.saturating_add(1)),
            );
            Some(VisibleCellText {
                wrap_text: text.contains('\n')
                    || text.contains('\r')
                    || alignment
                        .is_some_and(|alignment| alignment.wrap_text && !alignment.shrink_to_fit),
                text,
                vertical,
                cell_clip,
                blocks_left,
                blocks_right,
            })
        })
        .collect()
}

fn visible_indices(start: u32, indices: Option<&[u32]>, offset_count: usize) -> Vec<u32> {
    match indices {
        Some(indices) => indices.to_vec(),
        None => {
            let count = offset_count.saturating_sub(1) as u32;
            (start..start.saturating_add(count)).collect()
        }
    }
}

fn visible_ranges(indices: &[u32]) -> Vec<Range<u32>> {
    let mut ranges: Vec<Range<u32>> = Vec::new();
    for index in indices {
        if let Some(last) = ranges.last_mut() {
            if last.end == *index {
                last.end = last.end.saturating_add(1);
                continue;
            }
        }
        ranges.push(*index..index.saturating_add(1));
    }
    ranges
}

fn cell_bounds(
    grid: &xlsx_render::GridMeta,
    rows: &[u32],
    cols: &[u32],
    cell: CellRef,
) -> Option<Rect> {
    let row = rows.binary_search(&cell.row).ok()?;
    let col = cols.binary_search(&cell.col).ok()?;
    let left = *grid.col_offsets.get(col)?;
    let right = *grid.col_offsets.get(col + 1)?;
    let top = *grid.row_offsets.get(row)?;
    let bottom = *grid.row_offsets.get(row + 1)?;
    Some(Rect {
        x: left,
        y: top,
        w: right - left,
        h: bottom - top,
    })
}

fn has_blocking_neighbor(sheet: &xlsx_model::workbook::Sheet, cell: CellRef) -> bool {
    sheet.cell(cell).is_some()
        || sheet.col_style(cell.col).is_some()
        || sheet.merges.iter().any(|merge| merge.contains(cell))
}

fn rewrite_text_commands(commands: &mut Vec<DrawCmd>, cells: &[VisibleCellText]) {
    let original = std::mem::take(commands);
    let mut rewritten = Vec::with_capacity(original.len());
    let mut cell_index = 0;
    for mut command in original {
        let cell = match &command {
            DrawCmd::Text {
                text, chart: false, ..
            } => {
                let cell = cells.get(cell_index);
                if cell.is_some() {
                    cell_index += 1;
                }
                cell.filter(|cell| cell.text == text.as_ref())
            }
            _ => None,
        };
        if let Some(cell) = cell {
            if let Some(lines) = wrap_text_command(&command, cell) {
                rewritten.extend(lines);
                continue;
            }
            clip_text_spill(&mut command, cell);
        }
        rewritten.push(command);
    }
    *commands = rewritten;
}

fn clip_text_spill(command: &mut DrawCmd, cell: &VisibleCellText) {
    let (Some(cell_clip), DrawCmd::Text { clip, align, .. }) = (cell.cell_clip, command) else {
        return;
    };
    let blocked = match align {
        Align::Left => cell.blocks_right,
        Align::Right => cell.blocks_left,
        Align::Center => cell.blocks_left || cell.blocks_right,
    };
    if blocked {
        *clip = cell_clip;
    }
}

fn wrap_text_command(command: &DrawCmd, cell: &VisibleCellText) -> Option<Vec<DrawCmd>> {
    let DrawCmd::Text {
        y,
        text,
        font_size,
        clip,
        ..
    } = command
    else {
        return None;
    };
    if !cell.wrap_text {
        return None;
    }
    let available_width = (clip.w - 4.0).max(1.0);
    let lines = wrap_text_lines(text, available_width, *font_size);
    if lines.len() <= 1 {
        return None;
    }

    let line_height = (*font_size * 96.0 / 72.0 * 0.9 + 2.0).max(1.0);
    let max_lines = ((clip.h.max(0.0) / line_height).ceil() as usize + 1).clamp(1, 512);
    let visible_count = lines.len().min(max_lines);
    let start = match cell.vertical {
        VerticalAlignment::Top => 0,
        VerticalAlignment::Center => (lines.len() - visible_count) / 2,
        VerticalAlignment::Bottom => lines.len() - visible_count,
    };
    let first_baseline = match cell.vertical {
        VerticalAlignment::Top => *y,
        VerticalAlignment::Center => {
            *y - line_height * (visible_count.saturating_sub(1) as f32) / 2.0
        }
        VerticalAlignment::Bottom => *y - line_height * visible_count.saturating_sub(1) as f32,
    };
    Some(
        lines
            .into_iter()
            .skip(start)
            .take(visible_count)
            .enumerate()
            .map(|(index, line)| {
                let mut command = command.clone();
                if let DrawCmd::Text { y, text, .. } = &mut command {
                    *y = first_baseline + line_height * index as f32;
                    *text = line.into();
                }
                command
            })
            .collect(),
    )
}

fn wrap_text_lines(text: &str, available_width: f32, font_size: f32) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let paragraph = paragraph.strip_suffix('\r').unwrap_or(paragraph);
        if paragraph.trim().is_empty() {
            lines.push(paragraph.to_owned());
            continue;
        }
        let mut current = String::new();
        for word in paragraph.split_whitespace() {
            let candidate = if current.is_empty() {
                word.to_owned()
            } else {
                format!("{current} {word}")
            };
            if xlsx_raster::measure_text(&candidate, font_size) <= available_width {
                current = candidate;
                continue;
            }
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
            }
            current = word.to_owned();
        }
        if !current.is_empty() {
            lines.push(current);
        }
    }
    lines
}

#[cfg(test)]
mod tests;
