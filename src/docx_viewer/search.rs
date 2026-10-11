use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender};
use docx_layout::display_list::{DisplayList, DisplayPage, Primitive};
use eframe::egui;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct SearchBounds {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl SearchBounds {
    pub(super) fn new(left: f32, top: f32, right: f32, bottom: f32) -> Option<Self> {
        (left.is_finite()
            && top.is_finite()
            && right.is_finite()
            && bottom.is_finite()
            && left <= right
            && top <= bottom)
            .then_some(Self {
                left,
                top,
                right,
                bottom,
            })
    }

    pub(super) fn overlaps(self, other: Self) -> bool {
        self.left <= other.right
            && self.right >= other.left
            && self.top <= other.bottom
            && self.bottom >= other.top
    }

    fn union(self, other: Self) -> Self {
        Self {
            left: self.left.min(other.left),
            top: self.top.min(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct SearchMatch {
    pub page_index: usize,
    pub bounds: Option<SearchBounds>,
}

pub(super) struct SearchRequest {
    pub query: String,
    pub generation: u64,
}

pub(super) struct SearchResult {
    pub query: String,
    pub generation: u64,
    pub matches: Vec<SearchMatch>,
}

#[derive(Clone, Copy)]
enum PrimitiveArea {
    Body,
    NoteSeparator(usize),
    Note(usize),
    Header,
    Footer,
}

struct PrimitiveLocation {
    area: PrimitiveArea,
    index: usize,
}

struct IndexedPrimitive {
    location: PrimitiveLocation,
    paragraph_key: Option<String>,
    hidden: bool,
}

type TextStream = Vec<IndexedPrimitive>;

pub(super) struct DocxSearchIndex {
    display_list: Arc<DisplayList>,
    pages: Vec<Vec<TextStream>>,
}

impl DocxSearchIndex {
    pub fn new(display_list: Arc<DisplayList>) -> Self {
        let pages = display_list.pages.iter().map(index_page).collect();
        Self {
            display_list,
            pages,
        }
    }

    fn search(
        &self,
        query: &str,
        should_interrupt: &mut impl FnMut() -> bool,
    ) -> Option<Vec<SearchMatch>> {
        let query = query
            .chars()
            .flat_map(char::to_lowercase)
            .collect::<Vec<_>>();
        if query.is_empty() {
            return Some(Vec::new());
        }

        let prefix = prefix_table(&query);
        let mut matches = Vec::new();
        let mut work_since_poll = 0usize;

        for (page_index, streams) in self.pages.iter().enumerate() {
            if should_interrupt() {
                return None;
            }
            let page = &self.display_list.pages[page_index];
            for stream in streams {
                let mut matched = 0usize;
                let mut window = VecDeque::with_capacity(query.len());
                let mut previous_paragraph: Option<&str> = None;
                let mut had_previous = false;

                for indexed in stream {
                    if should_interrupt() {
                        return None;
                    }
                    if indexed.hidden {
                        matched = 0;
                        window.clear();
                        previous_paragraph = None;
                        had_previous = false;
                        continue;
                    }
                    let paragraph = indexed.paragraph_key.as_deref();
                    if had_previous
                        && (paragraph.is_none()
                            || previous_paragraph.is_none()
                            || paragraph != previous_paragraph)
                    {
                        matched = 0;
                        window.clear();
                    }
                    previous_paragraph = paragraph;
                    had_previous = true;

                    let primitive = primitive_at(page, &indexed.location);
                    let Some(characters) = primitive_characters(primitive) else {
                        continue;
                    };
                    for (character, bounds) in characters {
                        for normalized in character.to_lowercase() {
                            while matched > 0 && query[matched] != normalized {
                                matched = prefix[matched - 1];
                            }
                            if query[matched] == normalized {
                                matched += 1;
                            }

                            window.push_back(bounds);
                            if window.len() > query.len() {
                                window.pop_front();
                            }

                            if matched == query.len() {
                                let bounds = window
                                    .iter()
                                    .filter_map(|bounds| *bounds)
                                    .reduce(SearchBounds::union);
                                matches.push(SearchMatch { page_index, bounds });
                                matched = prefix[matched - 1];
                            }

                            work_since_poll += 1;
                            if work_since_poll >= 256 {
                                work_since_poll = 0;
                                if should_interrupt() {
                                    return None;
                                }
                            }
                        }
                    }
                }
            }
        }

        Some(matches)
    }
}

fn index_page(page: &DisplayPage) -> Vec<TextStream> {
    let mut streams = Vec::new();
    push_stream(&mut streams, &page.primitives, PrimitiveArea::Body);

    for (area_index, area) in page.note_areas.iter().enumerate() {
        let mut stream = index_primitives(
            &area.separator_primitives,
            PrimitiveArea::NoteSeparator(area_index),
        );
        stream.extend(index_primitives(
            &area.primitives,
            PrimitiveArea::Note(area_index),
        ));
        if !stream.is_empty() {
            streams.push(stream);
        }
    }

    if let Some(header) = &page.header {
        push_stream(&mut streams, &header.primitives, PrimitiveArea::Header);
    }
    if let Some(footer) = &page.footer {
        push_stream(&mut streams, &footer.primitives, PrimitiveArea::Footer);
    }
    streams
}

fn push_stream(streams: &mut Vec<TextStream>, primitives: &[Primitive], area: PrimitiveArea) {
    let stream = index_primitives(primitives, area);
    if !stream.is_empty() {
        streams.push(stream);
    }
}

fn index_primitives(primitives: &[Primitive], area: PrimitiveArea) -> TextStream {
    primitives
        .iter()
        .enumerate()
        .filter_map(|(index, primitive)| {
            let (paragraph_key, hidden) = primitive_metadata(primitive)?;
            Some(IndexedPrimitive {
                location: PrimitiveLocation { area, index },
                paragraph_key,
                hidden,
            })
        })
        .collect()
}

fn primitive_metadata(primitive: &Primitive) -> Option<(Option<String>, bool)> {
    let (attrs, hidden) = match primitive {
        Primitive::Text(run) => (&run.attrs, run.hidden),
        Primitive::GlyphRun(run) => (&run.attrs, run.hidden),
        _ => return None,
    };
    let paragraph_key = attrs
        .para_id
        .as_ref()
        .map(|value| format!("p:{value}"))
        .or_else(|| attrs.block_key.as_ref().map(|value| format!("k:{value}")))
        .or_else(|| attrs.block_id.as_ref().map(|value| format!("b:{value}")));
    Some((paragraph_key, hidden))
}

fn primitive_at<'a>(page: &'a DisplayPage, location: &PrimitiveLocation) -> &'a Primitive {
    match location.area {
        PrimitiveArea::Body => &page.primitives[location.index],
        PrimitiveArea::NoteSeparator(area_index) => {
            &page.note_areas[area_index].separator_primitives[location.index]
        }
        PrimitiveArea::Note(area_index) => &page.note_areas[area_index].primitives[location.index],
        PrimitiveArea::Header => {
            &page.header.as_ref().expect("indexed header").primitives[location.index]
        }
        PrimitiveArea::Footer => {
            &page.footer.as_ref().expect("indexed footer").primitives[location.index]
        }
    }
}

fn primitive_characters(primitive: &Primitive) -> Option<Vec<(char, Option<SearchBounds>)>> {
    match primitive {
        Primitive::Text(run) => {
            let bounds = text_run_bounds(run);
            Some(
                run.text
                    .chars()
                    .map(|character| (character, bounds))
                    .collect(),
            )
        }
        Primitive::GlyphRun(run) => {
            let bounds = glyph_character_bounds(run);
            Some(run.text.chars().zip(bounds).collect())
        }
        _ => None,
    }
}

pub(super) fn text_run_bounds(
    run: &docx_layout::display_list::TextRunPrimitive,
) -> Option<SearchBounds> {
    let left = run.x.as_f64()? as f32;
    let baseline = run.baseline_y.as_f64()? as f32;
    let width = run.width.as_f64()? as f32;
    let size = css_font_size(&run.font)?;
    SearchBounds::new(
        left,
        baseline - size * 0.8,
        left + width,
        baseline + size * 0.25,
    )
}

fn css_font_size(font: &str) -> Option<f32> {
    font.split_whitespace().find_map(|part| {
        part.strip_suffix("px")
            .and_then(|size| size.parse::<f32>().ok())
            .filter(|size| size.is_finite() && *size > 0.0)
    })
}

pub(super) fn glyph_character_bounds(
    run: &docx_layout::display_list::GlyphRunPrimitive,
) -> Vec<Option<SearchBounds>> {
    let size = run.size as f32;
    let mut clusters = BTreeMap::<u32, SearchBounds>::new();
    if size.is_finite() && size > 0.0 {
        for glyph in &run.glyphs {
            let x = glyph.x as f32;
            let next_x = (glyph.x + glyph.advance) as f32;
            let baseline = glyph.y as f32;
            let Some(bounds) = SearchBounds::new(
                x.min(next_x),
                baseline - size * 0.8,
                x.max(next_x),
                baseline + size * 0.25,
            ) else {
                continue;
            };
            clusters
                .entry(glyph.cluster)
                .and_modify(|current| *current = current.union(bounds))
                .or_insert(bounds);
        }
    }

    let fallback = clusters
        .values()
        .copied()
        .reduce(SearchBounds::union)
        .or_else(|| clip_bounds(run.paint_clip.as_ref()));

    run.text
        .char_indices()
        .map(|(byte_index, _)| {
            u32::try_from(byte_index)
                .ok()
                .and_then(|byte_index| clusters.range(..=byte_index).next_back())
                .map(|(_, bounds)| *bounds)
                .or(fallback)
        })
        .collect()
}

fn clip_bounds(clip: Option<&docx_layout::display_list::ClipRect>) -> Option<SearchBounds> {
    let clip = clip?;
    let left = clip.x.as_ref()?.as_f64()? as f32;
    let top = clip.y.as_ref()?.as_f64()? as f32;
    let width = clip.w.as_ref()?.as_f64()? as f32;
    let height = clip.h.as_ref()?.as_f64()? as f32;
    SearchBounds::new(left, top, left + width, top + height)
}

fn prefix_table(query: &[char]) -> Vec<usize> {
    let mut prefix = vec![0; query.len()];
    let mut matched = 0;
    for index in 1..query.len() {
        while matched > 0 && query[index] != query[matched] {
            matched = prefix[matched - 1];
        }
        if query[index] == query[matched] {
            matched += 1;
        }
        prefix[index] = matched;
    }
    prefix
}

pub(super) fn run_search_worker(
    index: DocxSearchIndex,
    request_rx: Receiver<SearchRequest>,
    result_tx: Sender<SearchResult>,
    repaint: egui::Context,
) {
    while let Ok(first) = request_rx.recv() {
        let mut request = latest_request(first, &request_rx);
        loop {
            if request.query.trim().is_empty() {
                break;
            }

            let mut replacement = None;
            let matches = index.search(&request.query, &mut || match request_rx.try_recv() {
                Ok(next) => {
                    replacement = Some(next);
                    true
                }
                Err(_) => false,
            });
            if let Some(next) = replacement {
                request = latest_request(next, &request_rx);
                continue;
            }
            let Some(matches) = matches else {
                break;
            };
            if result_tx
                .send(SearchResult {
                    query: request.query.clone(),
                    generation: request.generation,
                    matches,
                })
                .is_err()
            {
                return;
            }
            repaint.request_repaint();
            break;
        }
    }
}

fn latest_request(first: SearchRequest, rx: &Receiver<SearchRequest>) -> SearchRequest {
    let mut latest = first;
    while let Ok(next) = rx.try_recv() {
        latest = next;
    }
    latest
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn glyph_run(text: &str, x: f64, paragraph: &str) -> Value {
        let glyphs = text
            .char_indices()
            .enumerate()
            .map(|(index, (cluster, _))| {
                json!({
                    "id": index,
                    "x": x + index as f64 * 10.0,
                    "y": 20,
                    "cluster": cluster,
                    "advance": 10
                })
            })
            .collect::<Vec<_>>();
        json!({
            "kind": "glyphRun",
            "fontId": 0,
            "size": 10,
            "color": "#000000",
            "text": text,
            "glyphs": glyphs,
            "paraId": paragraph
        })
    }

    fn index_page(page: Value) -> DocxSearchIndex {
        let display_list: DisplayList = serde_json::from_value(json!({ "pages": [page] })).unwrap();
        DocxSearchIndex::new(Arc::new(display_list))
    }

    fn index(primitives: Vec<Value>) -> DocxSearchIndex {
        index_page(json!({
            "pageIndex": 0,
            "width": 100,
            "height": 100,
            "primitives": primitives
        }))
    }

    fn search(index: &DocxSearchIndex, query: &str) -> Vec<SearchMatch> {
        index.search(query, &mut || false).unwrap()
    }

    #[test]
    fn searches_notes_headers_and_footers() {
        let index = index_page(json!({
            "pageIndex": 0,
            "width": 100,
            "height": 100,
            "primitives": [],
            "noteAreas": [{
                "kind": "footnote",
                "primitives": [glyph_run("note word", 0.0, "note")]
            }],
            "header": {
                "rId": "header",
                "kind": "header",
                "y": 0,
                "height": 10,
                "primitives": [glyph_run("header word", 0.0, "header")]
            },
            "footer": {
                "rId": "footer",
                "kind": "footer",
                "y": 90,
                "height": 10,
                "primitives": [glyph_run("footer word", 0.0, "footer")]
            }
        }));

        assert_eq!(search(&index, "note").len(), 1);
        assert_eq!(search(&index, "header").len(), 1);
        assert_eq!(search(&index, "footer").len(), 1);
    }

    #[test]
    fn matches_case_insensitively_and_uses_glyph_geometry() {
        let index = index(vec![glyph_run("Find target, TARGET", 0.0, "p1")]);

        let matches = search(&index, "target");

        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].page_index, 0);
        assert_eq!(matches[0].bounds.unwrap().left, 50.0);
        assert_eq!(matches[0].bounds.unwrap().right, 110.0);
        assert_eq!(matches[1].bounds.unwrap().left, 130.0);
    }

    #[test]
    fn matches_across_runs_in_one_paragraph() {
        let index = index(vec![
            glyph_run("tar", 0.0, "p1"),
            glyph_run("get", 30.0, "p1"),
        ]);

        let matches = search(&index, "target");

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].bounds.unwrap().left, 0.0);
        assert_eq!(matches[0].bounds.unwrap().right, 60.0);
    }

    #[test]
    fn does_not_match_across_paragraphs() {
        let index = index(vec![
            glyph_run("tar", 0.0, "p1"),
            glyph_run("get", 30.0, "p2"),
        ]);

        assert!(search(&index, "target").is_empty());
    }

    #[test]
    fn supports_overlapping_occurrences() {
        let index = index(vec![glyph_run("aaaa", 0.0, "p1")]);

        assert_eq!(search(&index, "aa").len(), 3);
    }

    #[test]
    fn interrupts_search_when_a_new_request_arrives() {
        let index = index(vec![
            glyph_run("one", 0.0, "p1"),
            glyph_run("two", 40.0, "p1"),
        ]);
        let mut polls = 0;

        let result = index.search("missing", &mut || {
            polls += 1;
            polls > 1
        });

        assert!(result.is_none());
    }
}
