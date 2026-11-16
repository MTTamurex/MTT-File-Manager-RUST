use std::path::Path;
use std::sync::Arc;

use docx_edit::{seed_from_docx, EngineSession};
use docx_layout::display_list::DisplayList;
use docx_parse::s9::{parse_docx_s9_wire_with_limits, S9PackageWire, S9ParseOptions};
use docx_parse::xml::ParseLimits;
use docx_raster::{render_page_cached, GlyphCache, ImageCache, RenderResources};
use serde_json::{json, Value};

use crate::infrastructure::diagnostic_logger::docx_trace::{OpenTrace, Phase};

use super::fonts::RegisteredFonts;
use super::images;
use super::raster_scale::{high_resolution_scale, scale_display_page};
use super::search::SearchBounds;

mod display_build;

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
    pub fn open(path: &Path, trace: OpenTrace) -> Result<Self, String> {
        let mut stages = trace.stages(Phase::ReadFile);
        let bytes = std::fs::read(path)
            .map_err(|error| rust_i18n::t!("docxviewer.read_failed", error = error).to_string())?;
        stages.lap(Phase::ParsePackage);
        let limits = ParseLimits::default();
        let parsed = parse_docx_s9_wire_with_limits(&bytes, S9ParseOptions::default(), &limits)
            .map_err(|error| rust_i18n::t!("docxviewer.open_failed", error = error).to_string())?;
        let package = parsed.document.package;

        stages.lap(Phase::SeedEngine);
        let engine = EngineSession::new(0x4d54_444f_4358);
        seed_from_docx(engine.doc(), &bytes)
            .map_err(|error| rust_i18n::t!("docxviewer.open_failed", error = error).to_string())?;

        stages.lap(Phase::FontRequirements);
        let probe = region_request(&package, None)?;
        let requirements_json = engine
            .layout_font_requirements_json(&probe.to_string())
            .map_err(|error| rust_i18n::t!("docxviewer.open_failed", error = error).to_string())?;
        let requirements: Value =
            serde_json::from_str(&requirements_json).map_err(|error| error.to_string())?;
        stages.lap(Phase::LoadFonts);
        let fonts = RegisteredFonts::load(&requirements)?;
        stages.lap(Phase::LayoutRequest);
        let request = region_request(&package, Some(&fonts.layout_chain_ids))?;
        let layout_request = request.to_string();

        stages.lap(Phase::LayoutDocument);
        let region_layout = engine
            .layout_document_with_regions_retained_json(&layout_request)
            .map_err(|error| rust_i18n::t!("docxviewer.open_failed", error = error).to_string())?;
        stages.lap(Phase::BuildDisplayList);
        let display_extras = display_extras(&region_layout, &fonts.layout_chain_ids)?;
        drop(region_layout);
        display_build::build_frame(&engine, &display_extras, trace)
            .map_err(|error| rust_i18n::t!("docxviewer.open_failed", error = error).to_string())?;
        stages.lap(Phase::CloneDisplayList);
        let display_list = Arc::new(
            engine
                .with_display_list(Clone::clone)
                .ok_or_else(|| rust_i18n::t!("docxviewer.layout_failed").to_string())?,
        );

        stages.lap(Phase::PageSizes);
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

        stages.lap(Phase::LoadImages);
        let image_map = images::load_images(&package)?;
        stages.lap(Phase::FinalizeOpen);
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

    pub fn render_page(
        &mut self,
        page_index: usize,
        trace: OpenTrace,
    ) -> Result<DocxPagePixels, String> {
        let mut stages = trace.stages(Phase::ScalePage);
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
        stages.lap(Phase::RasterizePage);
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
        stages.lap(Phase::DecodePagePixels);
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

fn display_extras(
    region_layout: &str,
    font_chains: &std::collections::BTreeMap<String, Vec<u32>>,
) -> Result<String, String> {
    // The full region pass already resolves page fields, measures header/footer
    // stories and attaches notes to the retained layout. Reuse its payload
    // rather than invoking the edit/re-layout path just to recover these extras.
    // Ignore the serialized layout itself instead of allocating a second tree.
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct RegionReply {
        headers_footers: Option<Value>,
    }
    let reply: RegionReply =
        serde_json::from_str(region_layout).map_err(|error| error.to_string())?;
    let mut extras = json!({ "fontChains": font_chains });
    if let Some(headers_footers) = reply.headers_footers {
        extras["headersFooters"] = headers_footers;
    }
    Ok(extras.to_string())
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
mod tests;
