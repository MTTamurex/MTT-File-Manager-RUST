use std::path::Path;
use std::sync::Arc;

use docx_edit::{seed_from_docx, EngineSession};
use docx_layout::display_list::DisplayList;
use docx_parse::s9::{parse_docx_s9_wire_with_limits, S9PackageWire, S9ParseOptions};
use docx_parse::xml::ParseLimits;
use docx_raster::{render_page_cached, GlyphCache, ImageCache, RenderResources};
use serde_json::{json, Value};

use super::fonts::RegisteredFonts;
use super::images;
use super::raster_scale::{high_resolution_scale, scale_display_page};
use super::search::SearchBounds;

const MAX_PAGE_SIDE: f64 = 16_384.0;

pub(super) struct DocxPagePixels {
    pub page_index: usize,
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
    pub skipped_images: usize,
}

pub(super) struct DocxRenderer {
    display_list: Arc<DisplayList>,
    page_sizes: Vec<(f32, f32)>,
    fonts: RegisteredFonts,
    images: docx_raster::ImageMap,
    glyph_cache: GlyphCache,
    image_cache: ImageCache,
}

impl DocxRenderer {
    pub fn open(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path)
            .map_err(|error| rust_i18n::t!("docxviewer.read_failed", error = error).to_string())?;
        let limits = ParseLimits::default();
        let parsed = parse_docx_s9_wire_with_limits(&bytes, S9ParseOptions::default(), &limits)
            .map_err(|error| rust_i18n::t!("docxviewer.open_failed", error = error).to_string())?;
        let package = parsed.document.package;

        let engine = EngineSession::new(0x4d54_444f_4358);
        seed_from_docx(engine.doc(), &bytes)
            .map_err(|error| rust_i18n::t!("docxviewer.open_failed", error = error).to_string())?;

        let probe = region_request(&package, None)?;
        let requirements_json = engine
            .layout_font_requirements_json(&probe.to_string())
            .map_err(|error| rust_i18n::t!("docxviewer.open_failed", error = error).to_string())?;
        let requirements: Value =
            serde_json::from_str(&requirements_json).map_err(|error| error.to_string())?;
        let fonts = RegisteredFonts::load(&requirements)?;
        let request = region_request(&package, Some(&fonts.layout_chain_ids))?;
        let layout_request = request.to_string();

        engine
            .layout_document_with_regions_retained_json(&layout_request)
            .map_err(|error| rust_i18n::t!("docxviewer.open_failed", error = error).to_string())?;
        let display_extras = json!({ "fontChains": fonts.layout_chain_ids }).to_string();
        engine
            .build_display_list_frame(&display_extras, 0)
            .map_err(|error| rust_i18n::t!("docxviewer.open_failed", error = error).to_string())?;
        engine
            .apply_and_layout("body", 0)
            .map_err(|error| rust_i18n::t!("docxviewer.open_failed", error = error).to_string())?;
        let display_list = Arc::new(
            engine
                .with_display_list(Clone::clone)
                .ok_or_else(|| rust_i18n::t!("docxviewer.layout_failed").to_string())?,
        );

        let page_sizes = display_list
            .pages
            .iter()
            .map(|page| {
                let width = page
                    .width
                    .as_f64()
                    .filter(|value| value.is_finite() && *value > 0.0 && *value <= MAX_PAGE_SIDE)
                    .ok_or_else(|| rust_i18n::t!("docxviewer.page_size_invalid").to_string())?;
                let height = page
                    .height
                    .as_f64()
                    .filter(|value| value.is_finite() && *value > 0.0 && *value <= MAX_PAGE_SIDE)
                    .ok_or_else(|| rust_i18n::t!("docxviewer.page_size_invalid").to_string())?;
                Ok((width as f32, height as f32))
            })
            .collect::<Result<Vec<_>, String>>()?;

        if page_sizes.is_empty() {
            return Err(rust_i18n::t!("docxviewer.no_pages").to_string());
        }

        let image_map = images::load_images(&package)?;
        Ok(Self {
            display_list,
            page_sizes,
            fonts,
            images: image_map,
            glyph_cache: GlyphCache::default(),
            image_cache: ImageCache::default(),
        })
    }

    pub fn page_sizes(&self) -> &[(f32, f32)] {
        &self.page_sizes
    }

    pub fn search_display_list(&self) -> Arc<DisplayList> {
        Arc::clone(&self.display_list)
    }

    pub fn selected_text(&self, page_index: usize, bounds: SearchBounds) -> String {
        super::text_selection::selected_text(&self.display_list, page_index, bounds)
    }

    pub fn trim_caches(&mut self) {
        self.glyph_cache = GlyphCache::default();
        self.image_cache = ImageCache::default();
    }

    pub fn render_page(&mut self, page_index: usize) -> Result<DocxPagePixels, String> {
        let source_page = self
            .display_list
            .pages
            .get(page_index)
            .ok_or_else(|| format!("page ordinal {page_index} is out of range"))?;
        let width = source_page
            .width
            .as_f64()
            .ok_or_else(|| "page width is not numeric".to_owned())?;
        let height = source_page
            .height
            .as_f64()
            .ok_or_else(|| "page height is not numeric".to_owned())?;
        let scale = high_resolution_scale(width, height);
        let page = scale_display_page(source_page, scale)?;
        let render_list = DisplayList {
            contract_version: self.display_list.contract_version,
            pages: vec![page],
        };
        let resources = RenderResources::new(&self.fonts.store, &self.fonts.chains, &self.images);
        let rendered = render_page_cached(
            &render_list,
            0,
            &resources,
            &mut self.glyph_cache,
            &mut self.image_cache,
        )
        .map_err(|error| {
            rust_i18n::t!(
                "docxviewer.page_render_failed",
                page = (page_index + 1).to_string(),
                error = error
            )
            .to_string()
        })?;
        let rgba = image::load_from_memory(&rendered.bytes)
            .map_err(|error| error.to_string())?
            .into_rgba8();
        let (width, height) = rgba.dimensions();

        Ok(DocxPagePixels {
            page_index,
            width: width as usize,
            height: height as usize,
            rgba: rgba.into_raw(),
            skipped_images: rendered.skipped_images,
        })
    }
}

fn region_request(
    package: &S9PackageWire,
    font_chains: Option<&std::collections::BTreeMap<String, Vec<u32>>>,
) -> Result<Value, String> {
    let body = &package.document;
    let mut sections = body
        .sections
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|section| {
            let properties =
                serde_json::to_value(&section.properties).map_err(|error| error.to_string())?;
            let section_id = section
                .id
                .clone()
                .or_else(|| properties["sectionId"].as_str().map(str::to_owned));
            Ok(json!({ "sectionId": section_id, "properties": properties }))
        })
        .collect::<Result<Vec<_>, String>>()?;

    let final_properties = body
        .final_section_properties
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(|error| error.to_string())?
        .unwrap_or_else(|| json!({}));
    sections.push(json!({
        "sectionId": final_properties["sectionId"].as_str(),
        "properties": final_properties
    }));

    let watermark = sections
        .last()
        .and_then(|section| section["properties"].get("watermark"))
        .cloned()
        .unwrap_or(Value::Null);
    let mut notes = Vec::new();
    append_notes(&mut notes, package.footnotes.as_deref(), "footnote");
    append_notes(&mut notes, package.endnotes.as_deref(), "endnote");

    let compatibility = &package.settings.compatibility_flags;
    let measurement = font_chains.map(|chains| {
        json!({
            "fontChains": chains,
            "defaults": { "fontSize": 11, "fontFamily": "Calibri" },
            "compat": {
                "noLeading": compatibility.no_leading,
                "doNotExpandShiftReturn": compatibility.do_not_expand_shift_return
            },
            "authoritativeShaping": true
        })
    });

    let mut request = json!({
        "bodyStory": "body",
        "options": { "pageGap": 24 },
        "regions": {
            "sections": sections,
            "settings": package.settings,
            "watermark": watermark
        },
        "notes": { "contents": notes },
        "renderEnv": {}
    });
    if let Some(measurement) = measurement {
        request["measurement"] = measurement;
    }
    Ok(request)
}

fn append_notes(target: &mut Vec<Value>, notes: Option<&[docx_parse::Note]>, kind: &str) {
    for note in notes.unwrap_or_default() {
        if !note.is_separator() {
            target.push(json!({
                "id": note.id as i64,
                "noteKind": kind,
                "height": 0
            }));
        }
    }
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use std::io::{Cursor, Write};

    use super::*;

    fn synthetic_docx() -> Vec<u8> {
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
  <Default Extension="png" ContentType="image/png"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>"#,
                ),
                (
                    "_rels/.rels",
                    r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"#,
                ),
                (
                    "word/_rels/document.xml.rels",
                    r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rIdImage" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/image1.png"/>
</Relationships>"#,
                ),
                (
                    "word/document.xml",
                    r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture">
  <w:body>
    <w:p w14:paraId="11111111"><w:r><w:t>DOCX viewer fixture</w:t></w:r></w:p>
    <w:p w14:paraId="22222222"><w:r><w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0"><wp:extent cx="914400" cy="914400"/><wp:docPr id="1" name="Fixture picture"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="0" name="Fixture image"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="rIdImage"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="914400"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>
    <w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/></w:sectPr>
  </w:body>
</w:document>"#,
                ),
            ] {
                archive.start_file(name, options).unwrap();
                archive.write_all(contents.as_bytes()).unwrap();
            }
            let mut image = Cursor::new(Vec::new());
            image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
                2,
                2,
                image::Rgba([240, 20, 20, 255]),
            ))
            .write_to(&mut image, image::ImageFormat::Png)
            .unwrap();
            archive
                .start_file("word/media/image1.png", options)
                .unwrap();
            archive.write_all(&image.into_inner()).unwrap();
            archive.finish().unwrap();
        }
        cursor.into_inner()
    }

    #[test]
    fn opens_and_rasterizes_a_synthetic_docx_page() {
        let source = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(source.path(), synthetic_docx()).unwrap();

        let mut renderer = DocxRenderer::open(source.path()).unwrap();
        assert_eq!(renderer.page_sizes().len(), 1);
        let page = renderer.render_page(0).unwrap();

        let (layout_width, layout_height) = renderer.page_sizes()[0];
        assert_eq!(page.width, (layout_width * 2.0).ceil() as usize);
        assert_eq!(page.height, (layout_height * 2.0).ceil() as usize);
        assert!(page.width > 0);
        assert!(page.height > 0);
        assert_eq!(page.rgba.len(), page.width * page.height * 4);
        assert_eq!(page.skipped_images, 0);
        assert!(page
            .rgba
            .chunks_exact(4)
            .any(|pixel| pixel[0] < 128 && pixel[1] < 128 && pixel[2] < 128));
        assert!(page
            .rgba
            .chunks_exact(4)
            .any(|pixel| pixel[0] > 200 && pixel[1] < 80 && pixel[2] < 80));
    }
}
