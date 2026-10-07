use std::ops::Range;

use betteroffice_xlsx::{DrawCmd, SheetId, Workbook};
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
    let row_ranges = visible_ranges(
        display_list.grid.start_row,
        display_list.grid.row_indices.as_deref(),
        display_list.grid.row_offsets.len(),
    );
    let col_ranges = visible_ranges(
        display_list.grid.start_col,
        display_list.grid.col_indices.as_deref(),
        display_list.grid.col_offsets.len(),
    );
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
            if sheet
                .merges
                .iter()
                .any(|merge| merge.contains(cell_ref) && merge.start != cell_ref)
            {
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
            Some(VisibleCellText {
                wrap_text: text.contains('\n')
                    || text.contains('\r')
                    || alignment
                        .is_some_and(|alignment| alignment.wrap_text && !alignment.shrink_to_fit),
                text,
                vertical,
            })
        })
        .collect()
}

fn visible_ranges(start: u32, indices: Option<&[u32]>, offset_count: usize) -> Vec<Range<u32>> {
    let indices = match indices {
        Some(indices) => indices.to_vec(),
        None => {
            let count = offset_count.saturating_sub(1) as u32;
            (start..start.saturating_add(count)).collect()
        }
    };
    let mut ranges: Vec<Range<u32>> = Vec::new();
    for index in indices {
        if let Some(last) = ranges.last_mut() {
            if last.end == index {
                last.end = last.end.saturating_add(1);
                continue;
            }
        }
        ranges.push(index..index.saturating_add(1));
    }
    ranges
}

fn rewrite_text_commands(commands: &mut Vec<DrawCmd>, cells: &[VisibleCellText]) {
    let original = std::mem::take(commands);
    let mut rewritten = Vec::with_capacity(original.len());
    let mut cell_index = 0;
    for command in original {
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
        }
        rewritten.push(command);
    }
    *commands = rewritten;
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

    let line_height = (*font_size * 96.0 / 72.0 * 1.2).max(1.0);
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
mod tests {
    use super::{wrap_text_command, VerticalAlignment, VisibleCellText};
    use betteroffice_xlsx::{
        Align, CellRef, DrawCmd, Rect, SheetId, Viewport, Workbook, WorkbookModel,
    };
    use std::sync::Arc;

    #[test]
    fn wraps_text_into_measured_lines_and_keeps_command_style() {
        let original = "Sem contar com despesas de mercado, gastos diversos de dia a dia, exames médicos, transporte para exames e consultas, gastos inesperados";
        let command = DrawCmd::Text {
            x: 176.0,
            y: 30.0,
            text: Arc::from(original),
            font_size: 18.0,
            color: Arc::from("#ff0000"),
            clip: Rect {
                x: 0.0,
                y: 0.0,
                w: 180.0,
                h: 400.0,
            },
            align: Align::Right,
            bold: false,
            italic: true,
            underline: false,
            strike: false,
            highlight: None,
            dashed_underline: false,
            font_family: None,
            ghost: false,
            chart: false,
        };
        let cell = VisibleCellText {
            text: original.to_owned(),
            wrap_text: true,
            vertical: VerticalAlignment::Top,
        };
        let lines = wrap_text_command(&command, &cell).unwrap();
        let rendered_lines = lines
            .iter()
            .filter_map(|command| match command {
                DrawCmd::Text {
                    text,
                    font_size,
                    italic,
                    align,
                    ..
                } => {
                    assert!(*italic);
                    assert_eq!(*align, Align::Right);
                    assert!(*font_size > 0.0);
                    Some(text.as_ref())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(rendered_lines.len() > 1);
        assert_eq!(rendered_lines.join(" "), original);
        assert!(rendered_lines
            .iter()
            .all(|line| xlsx_raster::measure_text(line, 18.0) <= 176.0));
    }

    #[test]
    fn unwrapped_cells_keep_the_original_single_line_command() {
        let command = DrawCmd::Text {
            x: 0.0,
            y: 20.0,
            text: Arc::from("single line"),
            font_size: 11.0,
            color: Arc::from("#000000"),
            clip: Rect {
                x: 0.0,
                y: 0.0,
                w: 100.0,
                h: 20.0,
            },
            align: Align::Left,
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            highlight: None,
            dashed_underline: false,
            font_family: None,
            ghost: false,
            chart: false,
        };
        let cell = VisibleCellText {
            text: "single line".to_owned(),
            wrap_text: false,
            vertical: VerticalAlignment::Bottom,
        };
        assert!(wrap_text_command(&command, &cell).is_none());
    }

    #[test]
    fn display_list_wraps_a_cell_marked_wrap_text() {
        let text = "Sem contar com despesas de mercado, gastos diversos de dia a dia, exames médicos, transporte para exames e consultas, gastos inesperados";
        let mut model = WorkbookModel::default();
        model.styles.cell_xfs.push(xlsx_model::styles::Xf {
            alignment: Some(xlsx_model::styles::Alignment {
                wrap_text: true,
                ..Default::default()
            }),
            ..Default::default()
        });
        let mut sheet = xlsx_model::workbook::Sheet::new("Sheet");
        sheet.row_heights.insert(0, 300.0);
        sheet.set_cell(
            CellRef::new(0, 0),
            xlsx_model::workbook::Cell {
                style: Some(0),
                value: xlsx_model::value::CellValue::Text {
                    value: text.to_owned(),
                },
                ..Default::default()
            },
        );
        sheet.set_cell(
            CellRef::new(0, 1),
            xlsx_model::workbook::Cell {
                value: xlsx_model::value::CellValue::Text {
                    value: "manual\nlinebreak".to_owned(),
                },
                ..Default::default()
            },
        );
        model.sheets.push(sheet);
        let workbook = Workbook::from_model(model).unwrap();
        let sheet_id = SheetId(0);
        let mut display_list = workbook
            .display_list_for(
                sheet_id,
                &Viewport {
                    x: 0.0,
                    y: 0.0,
                    width: 180.0,
                    height: 500.0,
                },
            )
            .unwrap();

        super::wrap_display_list(&workbook, sheet_id, &mut display_list);

        let lines = display_list
            .commands
            .iter()
            .filter_map(|command| match command {
                DrawCmd::Text {
                    text, chart: false, ..
                } => Some(text.as_ref()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(lines.len() > 1);
        let combined = lines.join(" ");
        assert!(combined.starts_with(text));
        assert!(combined.ends_with("manual linebreak"));
    }
}
