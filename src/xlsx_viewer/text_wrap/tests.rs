use super::*;
use betteroffice_xlsx::{
    Align, CellRef, DrawCmd, Rect, SheetId, Viewport, Workbook, WorkbookModel,
};
use std::sync::Arc;

fn make_text_command(text: &str, clip: Rect, align: Align) -> DrawCmd {
    DrawCmd::Text {
        x: clip.x + clip.w,
        y: clip.y + 20.0,
        text: Arc::from(text),
        font_size: 11.0,
        color: Arc::from("#000000"),
        clip,
        align,
        bold: false,
        italic: false,
        underline: false,
        strike: false,
        highlight: None,
        dashed_underline: false,
        font_family: None,
        ghost: false,
        chart: false,
    }
}

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
        cell_clip: None,
        blocks_left: false,
        blocks_right: false,
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
    let command = make_text_command(
        "single line",
        Rect {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 20.0,
        },
        Align::Left,
    );
    let cell = VisibleCellText {
        text: "single line".to_owned(),
        wrap_text: false,
        vertical: VerticalAlignment::Bottom,
        cell_clip: None,
        blocks_left: false,
        blocks_right: false,
    };
    assert!(wrap_text_command(&command, &cell).is_none());
}

#[test]
fn display_list_wraps_a_cell_marked_wrap_text_and_preserves_manual_breaks() {
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

    wrap_display_list(&workbook, sheet_id, &mut display_list);

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

#[test]
fn spill_stops_before_a_formatted_empty_neighbor() {
    let long_text = "R$2500 (Se dividir em 12 vezes sem contar juros que é cobrado)";
    let mut model = WorkbookModel::default();
    model
        .styles
        .cell_xfs
        .push(xlsx_model::styles::Xf::default());
    model.styles.cell_xfs.push(xlsx_model::styles::Xf {
        alignment: Some(xlsx_model::styles::Alignment {
            h: Some(xlsx_model::HAlign::Right),
            ..Default::default()
        }),
        ..Default::default()
    });
    let mut sheet = xlsx_model::workbook::Sheet::new("Sheet");
    sheet.set_cell(
        CellRef::new(0, 0),
        xlsx_model::workbook::Cell {
            style: Some(0),
            ..Default::default()
        },
    );
    sheet.set_cell(
        CellRef::new(0, 1),
        xlsx_model::workbook::Cell {
            style: Some(1),
            value: xlsx_model::value::CellValue::Text {
                value: long_text.to_owned(),
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
                width: 400.0,
                height: 80.0,
            },
        )
        .unwrap();
    let expected_clip = Rect {
        x: display_list.grid.col_offsets[1],
        y: display_list.grid.row_offsets[0],
        w: display_list.grid.col_offsets[2] - display_list.grid.col_offsets[1],
        h: display_list.grid.row_offsets[1] - display_list.grid.row_offsets[0],
    };
    let original_clip = display_list
        .commands
        .iter()
        .find_map(|command| match command {
            DrawCmd::Text { text, clip, .. } if text.as_ref() == long_text => Some(*clip),
            _ => None,
        })
        .unwrap();
    assert!(original_clip.x < expected_clip.x);

    wrap_display_list(&workbook, sheet_id, &mut display_list);

    let clipped = display_list
        .commands
        .iter()
        .find_map(|command| match command {
            DrawCmd::Text { text, clip, .. } if text.as_ref() == long_text => Some(*clip),
            _ => None,
        })
        .unwrap();
    assert_eq!(clipped, expected_clip);
}
