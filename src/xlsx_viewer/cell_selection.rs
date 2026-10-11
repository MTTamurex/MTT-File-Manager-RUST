use betteroffice_xlsx::{CellRange, CellRef, GridGeometry, SheetId, Workbook, MAX_COLS, MAX_ROWS};
use xlsx_render::display_text;

use super::renderer::XlsxCellBounds;

const MAX_SELECTION_CELLS: u64 = 100_000;

pub(super) struct CellSelection {
    pub sheet_index: usize,
    pub range: CellRange,
    pub bounds: XlsxCellBounds,
    pub text: String,
    pub frozen_width: f32,
    pub frozen_height: f32,
}

pub(super) fn select_range(
    workbook: &Workbook,
    sheet_index: usize,
    start: (f32, f32),
    end: (f32, f32),
) -> Result<CellSelection, String> {
    let sheet_id = SheetId(sheet_index as u32);
    let sheet = workbook
        .sheet(sheet_id)
        .map_err(|error| error.to_string())?;
    let model = workbook.model();
    let geometry = GridGeometry::new(sheet, &model.styles);
    let start = point_to_cell(&geometry, start);
    let end = point_to_cell(&geometry, end);
    let range = CellRange::new(start, end);
    let row_count = u64::from(range.end.row - range.start.row) + 1;
    let col_count = u64::from(range.end.col - range.start.col) + 1;
    if row_count.saturating_mul(col_count) > MAX_SELECTION_CELLS {
        return Err(rust_i18n::t!("xlsxviewer.selection_too_large").to_string());
    }

    let mut text = String::new();
    for row in range.start.row..=range.end.row {
        if row != range.start.row {
            text.push('\n');
        }
        for col in range.start.col..=range.end.col {
            if col != range.start.col {
                text.push('\t');
            }
            if let Some(cell) = sheet.cell(CellRef::new(row, col)) {
                text.push_str(&display_text(&model.styles, model.date_system, cell));
            }
        }
    }

    let (frozen_rows, frozen_cols) = sheet
        .freeze_pane
        .map_or((0, 0), |pane| (pane.rows, pane.cols));
    Ok(CellSelection {
        sheet_index,
        range,
        bounds: XlsxCellBounds {
            left: geometry.col_x(range.start.col),
            top: geometry.row_y(range.start.row),
            right: geometry.col_x(range.end.col.saturating_add(1)),
            bottom: geometry.row_y(range.end.row.saturating_add(1)),
        },
        text,
        frozen_width: geometry.col_x(frozen_cols),
        frozen_height: geometry.row_y(frozen_rows),
    })
}

fn point_to_cell(geometry: &GridGeometry, point: (f32, f32)) -> CellRef {
    let x = if point.0.is_finite() {
        point.0.max(0.0)
    } else {
        0.0
    };
    let y = if point.1.is_finite() {
        point.1.max(0.0)
    } else {
        0.0
    };
    CellRef::new(
        geometry.row_at_y(y).min(MAX_ROWS - 1),
        geometry.col_at_x(x).min(MAX_COLS - 1),
    )
}

#[cfg(test)]
mod tests {
    use super::select_range;
    use betteroffice_xlsx::{CellRef, GridGeometry, Sheet, Workbook, WorkbookModel};

    #[test]
    fn selected_cells_are_exported_as_a_tab_delimited_rectangle() {
        let mut sheet = Sheet::new("Sheet");
        sheet.set_cell(
            CellRef::new(0, 0),
            xlsx_model::workbook::Cell {
                value: xlsx_model::value::CellValue::Text {
                    value: "alpha".to_owned(),
                },
                ..Default::default()
            },
        );
        sheet.set_cell(
            CellRef::new(0, 2),
            xlsx_model::workbook::Cell {
                value: xlsx_model::value::CellValue::Number { value: 42.0 },
                ..Default::default()
            },
        );
        sheet.set_cell(
            CellRef::new(1, 0),
            xlsx_model::workbook::Cell {
                value: xlsx_model::value::CellValue::Text {
                    value: "beta".to_owned(),
                },
                ..Default::default()
            },
        );
        let mut model = WorkbookModel::default();
        model.sheets.push(sheet);
        let workbook = Workbook::from_model(model).unwrap();
        let geometry = GridGeometry::new(
            workbook.sheet(betteroffice_xlsx::SheetId(0)).unwrap(),
            &workbook.model().styles,
        );

        let selection = select_range(
            &workbook,
            0,
            (geometry.col_x(0) + 1.0, geometry.row_y(0) + 1.0),
            (geometry.col_x(2) + 1.0, geometry.row_y(1) + 1.0),
        )
        .unwrap();

        assert_eq!(selection.range.start, CellRef::new(0, 0));
        assert_eq!(selection.range.end, CellRef::new(1, 2));
        assert_eq!(selection.text, "alpha\t\t42\nbeta\t\t");
    }
}
