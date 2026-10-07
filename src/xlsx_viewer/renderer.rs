use std::path::Path;

use betteroffice_xlsx::{
    CellRef, Error as XlsxError, GridGeometry, RenderedPng, SheetId, SheetInfo, Viewport, Workbook,
    MAX_PIXMAP_DIM, MAX_PIXMAP_PIXELS,
};
use rust_i18n::t;

pub(super) struct OpenedWorkbook {
    pub sheet_info: SheetInfo,
}

pub(super) struct XlsxViewportTile {
    pub viewport: Viewport,
    pub body_offset_x: f32,
    pub body_offset_y: f32,
    pub frozen_width: f32,
    pub frozen_height: f32,
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

#[derive(Clone, Copy)]
pub(super) struct XlsxCellBounds {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

#[derive(Clone)]
pub(super) struct XlsxSearchMatch {
    pub sheet_index: usize,
    pub cell: CellRef,
    pub address: String,
    pub text: String,
    pub bounds: XlsxCellBounds,
    pub frozen_rows: u32,
    pub frozen_cols: u32,
    pub frozen_width: f32,
    pub frozen_height: f32,
    pub scroll_position: (f32, f32),
}

struct SearchSheetGeometry {
    sheet: SheetId,
    geometry: GridGeometry,
    frozen_rows: u32,
    frozen_cols: u32,
    frozen_width: f32,
    frozen_height: f32,
}

pub(super) struct XlsxRenderer {
    workbook: Workbook,
    sheet_count: usize,
}

impl XlsxRenderer {
    pub fn open(path: &Path) -> Result<(Self, OpenedWorkbook), String> {
        let bytes = std::fs::read(path)
            .map_err(|error| t!("xlsxviewer.read_failed", error = error).to_string())?;
        let workbook = Workbook::open_for_read(&bytes)
            .map_err(|error| t!("xlsxviewer.open_failed", error = error).to_string())?;
        let sheet_info = workbook
            .sheet_info()
            .map_err(|error| t!("xlsxviewer.open_failed", error = error).to_string())?;
        if sheet_info.sheet_names.is_empty() {
            return Err(t!("xlsxviewer.no_sheets").to_string());
        }

        let renderer = Self {
            workbook,
            sheet_count: sheet_info.sheet_names.len(),
        };
        Ok((renderer, OpenedWorkbook { sheet_info }))
    }

    pub fn select_sheet(&mut self, sheet_index: usize) -> Result<SheetInfo, String> {
        if sheet_index >= self.sheet_count {
            return Err(t!("xlsxviewer.sheet_unavailable").to_string());
        }
        self.workbook
            .set_active_sheet(SheetId(sheet_index as u32))
            .map_err(|error| t!("xlsxviewer.sheet_open_failed", error = error).to_string())?;
        self.workbook
            .sheet_info()
            .map_err(|error| t!("xlsxviewer.sheet_open_failed", error = error).to_string())
    }

    pub fn render_viewport(
        &self,
        sheet_index: usize,
        viewport: Viewport,
    ) -> Result<Vec<XlsxViewportTile>, String> {
        let sheet = SheetId(sheet_index as u32);
        let sheet_ref = self
            .workbook
            .sheet(sheet)
            .map_err(|error| t!("xlsxviewer.render_failed", error = error).to_string())?;
        let geometry = GridGeometry::new(sheet_ref, &self.workbook.model().styles);
        let (frozen_rows, frozen_cols) = sheet_ref
            .freeze_pane
            .map_or((0, 0), |pane| (pane.rows, pane.cols));
        let frozen_width = geometry.col_x(frozen_cols).min(viewport.width).max(0.0);
        let frozen_height = geometry.row_y(frozen_rows).min(viewport.height).max(0.0);
        render_viewport_tiles(viewport, frozen_width, frozen_height, |viewport| {
            self.render_tile(sheet, viewport)
        })
    }

    fn render_tile(&self, sheet: SheetId, viewport: Viewport) -> Result<RenderedPng, XlsxError> {
        let mut display_list = self.workbook.display_list_for(sheet, &viewport)?;
        super::text_wrap::wrap_display_list(&self.workbook, sheet, &mut display_list);

        let width = (display_list.width.ceil() as u32).max(1);
        let height = (display_list.height.ceil() as u32).max(1);
        if width > MAX_PIXMAP_DIM || height > MAX_PIXMAP_DIM {
            return Err(XlsxError::RenderTooLarge {
                width,
                height,
                max: MAX_PIXMAP_DIM,
            });
        }
        if u64::from(width) * u64::from(height) > MAX_PIXMAP_PIXELS {
            return Err(XlsxError::RenderAreaTooLarge {
                width,
                height,
                max_pixels: MAX_PIXMAP_PIXELS,
            });
        }

        let bytes = xlsx_raster::render_png(&display_list).map_err(XlsxError::Raster)?;
        Ok(RenderedPng {
            bytes,
            width,
            height,
        })
    }

    pub fn search(&self, query: &str) -> Vec<XlsxSearchMatch> {
        let styles = &self.workbook.model().styles;
        let mut current_geometry: Option<SearchSheetGeometry> = None;
        self.workbook
            .search_text(
                query,
                false,
                Some(betteroffice_xlsx::DEFAULT_TEXT_SEARCH_LIMIT),
            )
            .into_iter()
            .filter_map(|result| {
                let sheet_id = result.address.sheet;
                let sheet = self.workbook.sheet(sheet_id).ok()?;
                if current_geometry
                    .as_ref()
                    .is_none_or(|cached| cached.sheet != sheet_id)
                {
                    let geometry = GridGeometry::new(sheet, styles);
                    let (frozen_rows, frozen_cols) = sheet
                        .freeze_pane
                        .map_or((0, 0), |pane| (pane.rows, pane.cols));
                    current_geometry = Some(SearchSheetGeometry {
                        sheet: sheet_id,
                        frozen_width: geometry.col_x(frozen_cols),
                        frozen_height: geometry.row_y(frozen_rows),
                        frozen_rows,
                        frozen_cols,
                        geometry,
                    });
                }

                let geometry = current_geometry.as_ref()?;
                let cell = result.address.cell;
                let merged = sheet.merges.iter().find(|range| {
                    cell.row >= range.start.row
                        && cell.row <= range.end.row
                        && cell.col >= range.start.col
                        && cell.col <= range.end.col
                });
                let (start_row, start_col, end_row, end_col) =
                    merged.map_or((cell.row, cell.col, cell.row, cell.col), |range| {
                        (
                            range.start.row,
                            range.start.col,
                            range.end.row,
                            range.end.col,
                        )
                    });

                Some(XlsxSearchMatch {
                    sheet_index: sheet_id.0 as usize,
                    cell,
                    address: self.workbook.format_address(result.address),
                    text: result.text,
                    bounds: XlsxCellBounds {
                        left: geometry.geometry.col_x(start_col),
                        top: geometry.geometry.row_y(start_row),
                        right: geometry.geometry.col_x(end_col.saturating_add(1)),
                        bottom: geometry.geometry.row_y(end_row.saturating_add(1)),
                    },
                    frozen_rows: geometry.frozen_rows,
                    frozen_cols: geometry.frozen_cols,
                    frozen_width: geometry.frozen_width,
                    frozen_height: geometry.frozen_height,
                    scroll_position: (
                        (geometry.geometry.col_x(cell.col) - geometry.frozen_width).max(0.0),
                        (geometry.geometry.row_y(cell.row) - geometry.frozen_height).max(0.0),
                    ),
                })
            })
            .collect()
    }
}

const MAX_VIEWPORT_TILES: usize = 256;
const MAX_VIEWPORT_RENDER_ATTEMPTS: usize = MAX_VIEWPORT_TILES * 2;

#[derive(Clone, Copy)]
struct BodyRegion {
    offset_x: f32,
    offset_y: f32,
    width: f32,
    height: f32,
}

#[derive(Clone, Copy)]
struct BodyViewport {
    root: Viewport,
    frozen_width: f32,
    frozen_height: f32,
    offset_x: f32,
    offset_y: f32,
    width: f32,
    height: f32,
}

struct ViewportTiler<'a, F> {
    render: &'a mut F,
    tiles: Vec<XlsxViewportTile>,
    attempts: usize,
}

impl<F> ViewportTiler<'_, F>
where
    F: FnMut(Viewport) -> Result<RenderedPng, XlsxError>,
{
    fn render_body(&mut self, body: BodyViewport) -> Result<(), String> {
        if self.attempts >= MAX_VIEWPORT_RENDER_ATTEMPTS || self.tiles.len() >= MAX_VIEWPORT_TILES {
            return Err(t!("xlsxviewer.render_tile_limit").to_string());
        }
        self.attempts += 1;

        let viewport = Viewport {
            x: body.root.x + body.offset_x,
            y: body.root.y + body.offset_y,
            width: body.frozen_width + body.width,
            height: body.frozen_height + body.height,
        };
        let rendered = match (self.render)(viewport) {
            Ok(rendered) => rendered,
            Err(error) if is_render_size_limit(&error) => {
                let Some(regions) = split_body_region(body.width, body.height) else {
                    return Err(t!("xlsxviewer.render_failed", error = error).to_string());
                };
                for region in regions {
                    self.render_body(BodyViewport {
                        offset_x: body.offset_x + region.offset_x,
                        offset_y: body.offset_y + region.offset_y,
                        width: region.width,
                        height: region.height,
                        ..body
                    })?;
                }
                return Ok(());
            }
            Err(error) => return Err(t!("xlsxviewer.render_failed", error = error).to_string()),
        };

        let rgba = image::load_from_memory(&rendered.bytes)
            .map_err(|error| t!("xlsxviewer.render_failed", error = error).to_string())?
            .into_rgba8();
        let (width, height) = rgba.dimensions();
        self.tiles.push(XlsxViewportTile {
            viewport,
            body_offset_x: body.offset_x,
            body_offset_y: body.offset_y,
            frozen_width: body.frozen_width,
            frozen_height: body.frozen_height,
            width: width as usize,
            height: height as usize,
            rgba: rgba.into_raw(),
        });
        Ok(())
    }
}

fn render_viewport_tiles(
    viewport: Viewport,
    frozen_width: f32,
    frozen_height: f32,
    mut render: impl FnMut(Viewport) -> Result<RenderedPng, XlsxError>,
) -> Result<Vec<XlsxViewportTile>, String> {
    let frozen_width = frozen_width.min(viewport.width).max(0.0);
    let frozen_height = frozen_height.min(viewport.height).max(0.0);
    let mut tiler = ViewportTiler {
        render: &mut render,
        tiles: Vec::new(),
        attempts: 0,
    };
    tiler.render_body(BodyViewport {
        root: viewport,
        frozen_width,
        frozen_height,
        offset_x: 0.0,
        offset_y: 0.0,
        width: viewport.width - frozen_width,
        height: viewport.height - frozen_height,
    })?;
    Ok(tiler.tiles)
}

fn is_render_size_limit(error: &XlsxError) -> bool {
    matches!(
        error,
        XlsxError::DisplayTooLarge { .. }
            | XlsxError::RenderTooLarge { .. }
            | XlsxError::RenderAreaTooLarge { .. }
    )
}

fn split_body_region(width: f32, height: f32) -> Option<Vec<BodyRegion>> {
    let split_x = width >= 2.0;
    let split_y = height >= 2.0;
    match (split_x, split_y) {
        (true, true) => {
            let left_width = width / 2.0;
            let right_width = width - left_width;
            let top_height = height / 2.0;
            let bottom_height = height - top_height;
            Some(vec![
                BodyRegion {
                    offset_x: 0.0,
                    offset_y: 0.0,
                    width: left_width,
                    height: top_height,
                },
                BodyRegion {
                    offset_x: left_width,
                    offset_y: 0.0,
                    width: right_width,
                    height: top_height,
                },
                BodyRegion {
                    offset_x: 0.0,
                    offset_y: top_height,
                    width: left_width,
                    height: bottom_height,
                },
                BodyRegion {
                    offset_x: left_width,
                    offset_y: top_height,
                    width: right_width,
                    height: bottom_height,
                },
            ])
        }
        (true, false) => {
            let left_width = width / 2.0;
            Some(vec![
                BodyRegion {
                    offset_x: 0.0,
                    offset_y: 0.0,
                    width: left_width,
                    height,
                },
                BodyRegion {
                    offset_x: left_width,
                    offset_y: 0.0,
                    width: width - left_width,
                    height,
                },
            ])
        }
        (false, true) => {
            let top_height = height / 2.0;
            Some(vec![
                BodyRegion {
                    offset_x: 0.0,
                    offset_y: 0.0,
                    width,
                    height: top_height,
                },
                BodyRegion {
                    offset_x: 0.0,
                    offset_y: top_height,
                    width,
                    height: height - top_height,
                },
            ])
        }
        (false, false) => None,
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use betteroffice_xlsx::Viewport;

    use super::{render_viewport_tiles, XlsxRenderer};

    fn synthetic_xlsx() -> Vec<u8> {
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut archive = zip::ZipWriter::new(&mut cursor);
            let options = zip::write::SimpleFileOptions::default();
            for (name, contents) in [
                (
                    "[Content_Types].xml",
                    r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/>
</Types>"#,
                ),
                (
                    "_rels/.rels",
                    r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#,
                ),
                (
                    "xl/workbook.xml",
                    r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets>
    <sheet name="Summary" sheetId="1" r:id="rId1"/>
    <sheet name="Details" sheetId="2" r:id="rId2"/>
  </sheets>
</workbook>"#,
                ),
                (
                    "xl/_rels/workbook.xml.rels",
                    r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/>
</Relationships>"#,
                ),
                (
                    "xl/sharedStrings.xml",
                    r#"<?xml version="1.0" encoding="UTF-8"?>
<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="2" uniqueCount="2">
  <si><t>Search needle</t></si>
  <si><t>Second sheet value</t></si>
</sst>"#,
                ),
                (
                    "xl/worksheets/sheet1.xml",
                    r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1"><c r="A1" t="s"><v>0</v></c></row>
    <row r="2"><c r="A2"><v>42</v></c></row>
  </sheetData>
</worksheet>"#,
                ),
                (
                    "xl/worksheets/sheet2.xml",
                    r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1"><c r="A1" t="s"><v>1</v></c></row>
  </sheetData>
</worksheet>"#,
                ),
            ] {
                archive.start_file(name, options).unwrap();
                archive.write_all(contents.as_bytes()).unwrap();
            }
            archive.finish().unwrap();
        }
        cursor.into_inner()
    }

    #[test]
    fn opens_renders_and_searches_a_synthetic_workbook() {
        let source = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(source.path(), synthetic_xlsx()).unwrap();

        let (mut renderer, opened) = XlsxRenderer::open(source.path()).unwrap();
        assert_eq!(opened.sheet_info.sheet_names, ["Summary", "Details"]);
        assert_eq!(opened.sheet_info.active_sheet.0, 0);

        let tiles = renderer
            .render_viewport(
                0,
                Viewport {
                    x: 0.0,
                    y: 0.0,
                    width: 480.0,
                    height: 300.0,
                },
            )
            .unwrap();
        assert_eq!(tiles.len(), 1);
        assert!(tiles[0].width > 0);
        assert!(tiles[0].height > 0);
        assert_eq!(tiles[0].rgba.len(), tiles[0].width * tiles[0].height * 4);
        assert!(tiles[0].rgba.chunks_exact(4).any(|pixel| pixel[0] < 128));

        let matches = renderer.search("needle");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].sheet_index, 0);
        assert!(matches[0].address.contains("A1"));
        assert_eq!(matches[0].text, "Search needle");

        let second_sheet = renderer.select_sheet(1).unwrap();
        assert_eq!(second_sheet.active_sheet.0, 1);
        assert_eq!(second_sheet.sheet_names[1], "Details");
    }

    #[test]
    fn splits_viewports_that_exceed_the_display_cell_cap() {
        let viewport = Viewport {
            x: 0.0,
            y: 0.0,
            width: 1_000.0,
            height: 600.0,
        };
        let frozen_width = 80.0;
        let frozen_height = 40.0;
        let mut encoded_png = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            1,
            1,
            image::Rgba([255, 255, 255, 255]),
        ))
        .write_to(&mut encoded_png, image::ImageFormat::Png)
        .unwrap();
        let png = encoded_png.into_inner();
        let mut render_calls = 0;

        let tiles = render_viewport_tiles(viewport, frozen_width, frozen_height, |tile| {
            render_calls += 1;
            let cells = (tile.width.ceil() as u64).saturating_mul(tile.height.ceil() as u64);
            if cells > betteroffice_xlsx::MAX_DISPLAY_CELLS {
                Err(betteroffice_xlsx::Error::DisplayTooLarge {
                    cells,
                    max: betteroffice_xlsx::MAX_DISPLAY_CELLS,
                })
            } else {
                Ok(betteroffice_xlsx::RenderedPng {
                    bytes: png.clone(),
                    width: 1,
                    height: 1,
                })
            }
        })
        .unwrap();

        assert_eq!(tiles.len(), 4);
        assert_eq!(render_calls, 5);
        assert!(tiles.iter().all(|tile| {
            tile.frozen_width == frozen_width
                && tile.frozen_height == frozen_height
                && (tile.viewport.width.ceil() as u64)
                    .saturating_mul(tile.viewport.height.ceil() as u64)
                    <= betteroffice_xlsx::MAX_DISPLAY_CELLS
        }));
        let tiled_body_area = tiles
            .iter()
            .map(|tile| {
                (tile.viewport.width - tile.frozen_width)
                    * (tile.viewport.height - tile.frozen_height)
            })
            .sum::<f32>();
        let expected_body_area =
            (viewport.width - frozen_width) * (viewport.height - frozen_height);
        assert!((tiled_body_area - expected_body_area).abs() < 1.0);
    }
}
