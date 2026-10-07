use std::cmp::Reverse;
use std::collections::{BinaryHeap, VecDeque};
use std::ops::Range;

use eframe::egui;
use rust_i18n::t;

use super::TextViewerApp;

const CHECKPOINT_STRIDE: usize = 256;

#[derive(Default)]
pub(super) struct TextSearchState {
    pub open: bool,
    query: String,
    submitted_query: String,
    normalized_query: Vec<char>,
    occurrences: OccurrenceIndex,
    current: usize,
    focus_request: bool,
}

#[derive(Default)]
struct OccurrenceIndex {
    deltas: Vec<u8>,
    checkpoints: Vec<OccurrenceCheckpoint>,
    count: usize,
    previous_offset: u32,
}

#[derive(Clone, Copy)]
struct OccurrenceCheckpoint {
    match_index: usize,
    encoded_offset: usize,
    previous_offset: u32,
}

impl OccurrenceIndex {
    fn push(&mut self, offset: u32) {
        debug_assert!(self.count == 0 || offset >= self.previous_offset);
        if self.count == self.checkpoints.len() * CHECKPOINT_STRIDE {
            self.checkpoints.push(OccurrenceCheckpoint {
                match_index: self.count,
                encoded_offset: self.deltas.len(),
                previous_offset: self.previous_offset,
            });
        }
        encode_varint(offset - self.previous_offset, &mut self.deltas);
        self.previous_offset = offset;
        self.count += 1;
    }

    fn len(&self) -> usize {
        self.count
    }

    fn get(&self, match_index: usize) -> Option<u32> {
        if match_index >= self.count {
            return None;
        }
        let checkpoint = self.checkpoints[match_index / CHECKPOINT_STRIDE];
        let mut encoded_offset = checkpoint.encoded_offset;
        let mut offset = checkpoint.previous_offset;
        for _ in checkpoint.match_index..=match_index {
            offset = offset.checked_add(decode_varint(&self.deltas, &mut encoded_offset)?)?;
        }
        Some(offset)
    }

    fn lower_bound(&self, target: u32) -> usize {
        let mut first = 0;
        let mut last = self.count;
        while first < last {
            let middle = first + (last - first) / 2;
            if self.get(middle).expect("indexed occurrence") < target {
                first = middle + 1;
            } else {
                last = middle;
            }
        }
        first
    }
}

fn encode_varint(mut value: u32, target: &mut Vec<u8>) {
    while value >= 0x80 {
        target.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    target.push(value as u8);
}

fn decode_varint(bytes: &[u8], cursor: &mut usize) -> Option<u32> {
    let mut value = 0u32;
    let mut shift = 0;
    loop {
        let byte = *bytes.get(*cursor)?;
        *cursor += 1;
        value |= u32::from(byte & 0x7f).checked_shl(shift)?;
        if byte & 0x80 == 0 {
            return Some(value);
        }
        shift += 7;
        if shift >= 32 {
            return None;
        }
    }
}

struct LineMatchSearch<'a> {
    line: &'a str,
    line_start: u32,
    occurrences: &'a OccurrenceIndex,
    query: &'a [char],
    match_range: Range<usize>,
    current: usize,
}

impl TextSearchState {
    fn clear_results(&mut self) {
        self.submitted_query.clear();
        self.normalized_query.clear();
        self.occurrences = OccurrenceIndex::default();
        self.current = 0;
    }

    fn submit(&mut self, content: &str, line_offsets: &[u32]) -> Option<u32> {
        self.clear_results();
        let query = self.query.trim().to_owned();
        if query.is_empty() {
            return None;
        }
        let (occurrences, normalized_query) = find_occurrences(content, line_offsets, &query);
        self.submitted_query = query;
        self.normalized_query = normalized_query;
        self.occurrences = occurrences;
        self.occurrences.get(0)
    }

    fn next_offset(&mut self) -> Option<u32> {
        if self.occurrences.len() == 0 {
            return None;
        }
        self.current = (self.current + 1) % self.occurrences.len();
        self.occurrences.get(self.current)
    }

    fn previous_offset(&mut self) -> Option<u32> {
        if self.occurrences.len() == 0 {
            return None;
        }
        self.current = if self.current == 0 {
            self.occurrences.len() - 1
        } else {
            self.current - 1
        };
        self.occurrences.get(self.current)
    }
}

fn find_occurrences(
    content: &str,
    line_offsets: &[u32],
    query: &str,
) -> (OccurrenceIndex, Vec<char>) {
    let normalized_query = query
        .chars()
        .flat_map(char::to_lowercase)
        .collect::<Vec<_>>();
    let mut occurrences = OccurrenceIndex::default();
    if normalized_query.is_empty() {
        return (occurrences, normalized_query);
    }

    let prefix = prefix_table(&normalized_query);
    for (line_index, &line_start) in line_offsets.iter().enumerate() {
        let line = line_at(content, line_offsets, line_index);
        let mut matched = 0usize;
        let mut source_starts = VecDeque::with_capacity(normalized_query.len().min(4096));
        for (byte_index, character) in line.char_indices() {
            let absolute_offset = line_start + byte_index as u32;
            for normalized in character.to_lowercase() {
                while matched > 0 && normalized_query[matched] != normalized {
                    matched = prefix[matched - 1];
                }
                if normalized_query[matched] == normalized {
                    matched += 1;
                }
                source_starts.push_back(absolute_offset);
                if source_starts.len() > normalized_query.len() {
                    source_starts.pop_front();
                }
                if matched == normalized_query.len() {
                    occurrences.push(*source_starts.front().expect("matched source character"));
                    matched = prefix[matched - 1];
                }
            }
        }
    }

    (occurrences, normalized_query)
}

fn line_at<'a>(content: &'a str, line_offsets: &[u32], line_index: usize) -> &'a str {
    let start = line_offsets[line_index] as usize;
    let mut end = line_offsets
        .get(line_index + 1)
        .map(|offset| *offset as usize)
        .unwrap_or(content.len());
    if content.as_bytes().get(end.saturating_sub(1)) == Some(&b'\n') {
        end -= 1;
    }
    if content.as_bytes().get(end.saturating_sub(1)) == Some(&b'\r') {
        end -= 1;
    }
    &content[start..end]
}

fn prefix_table(query: &[char]) -> Vec<usize> {
    let mut prefix = vec![0; query.len()];
    let mut matched = 0usize;
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

fn matched_range(line: &str, start: usize, query: &[char]) -> Option<Range<usize>> {
    let text = line.get(start..)?;
    let mut matched = 0;
    for (relative_start, character) in text.char_indices() {
        for normalized in character.to_lowercase() {
            if query.get(matched).copied()? != normalized {
                return None;
            }
            matched += 1;
            if matched == query.len() {
                return Some(start..start + relative_start + character.len_utf8());
            }
        }
    }
    None
}

impl TextViewerApp {
    pub(super) fn search_is_open(&self) -> bool {
        self.search.open
    }

    pub(super) fn toggle_search(&mut self) {
        if self.search.open {
            self.close_search();
        } else {
            self.search.open = true;
            self.search.focus_request = true;
            self.goto_open = false;
        }
    }

    pub(super) fn close_search(&mut self) {
        self.search.open = false;
        self.search.focus_request = false;
        self.search.query.clear();
        self.search.clear_results();
        self.scroll_to_line = None;
    }

    fn navigate_to_offset(&mut self, offset: u32) {
        if self.line_offsets.is_empty() {
            return;
        }
        let line_index = self
            .line_offsets
            .partition_point(|line_start| *line_start <= offset)
            .saturating_sub(1);
        self.scroll_to_line = Some(line_index);
    }

    fn execute_search(&mut self) {
        let first = self.search.submit(&self.content, &self.line_offsets);
        if let Some(offset) = first {
            self.navigate_to_offset(offset);
        } else {
            self.scroll_to_line = None;
        }
    }

    fn next_search_match(&mut self) {
        if let Some(offset) = self.search.next_offset() {
            self.navigate_to_offset(offset);
        }
    }

    fn previous_search_match(&mut self) {
        if let Some(offset) = self.search.previous_offset() {
            self.navigate_to_offset(offset);
        }
    }

    pub(super) fn handle_search_shortcuts(&mut self, ctx: &egui::Context) {
        let toggle = ctx.input_mut(|input| {
            if input.modifiers.ctrl && !input.modifiers.shift && input.key_pressed(egui::Key::F) {
                if let Some(position) = input.events.iter().position(|event| {
                    matches!(
                        event,
                        egui::Event::Key {
                            key: egui::Key::F,
                            ..
                        }
                    )
                }) {
                    input.events.remove(position);
                }
                true
            } else {
                false
            }
        });
        if toggle {
            self.toggle_search();
        }

        if self.search.open
            && ctx.input_mut(|input| {
                if input.key_pressed(egui::Key::Escape) {
                    if let Some(position) = input.events.iter().position(|event| {
                        matches!(
                            event,
                            egui::Event::Key {
                                key: egui::Key::Escape,
                                ..
                            }
                        )
                    }) {
                        input.events.remove(position);
                    }
                    true
                } else {
                    false
                }
            })
        {
            self.close_search();
            return;
        }

        if self.search.open
            && ctx.input(|input| input.key_pressed(egui::Key::F3) && !input.modifiers.ctrl)
        {
            if ctx.input(|input| input.modifiers.shift) {
                self.previous_search_match();
            } else {
                self.next_search_match();
            }
        }
    }

    pub(super) fn show_search_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.search.query)
                    .desired_width(250.0)
                    .hint_text(t!("textviewer.search_placeholder").to_string()),
            );
            if self.search.focus_request {
                response.request_focus();
                self.search.focus_request = false;
            }

            if response.changed() && self.search.query.trim() != self.search.submitted_query {
                self.search.clear_results();
                self.scroll_to_line = None;
            }

            let enter_pressed = ui.input(|input| input.key_pressed(egui::Key::Enter));
            if enter_pressed && (response.has_focus() || response.lost_focus()) {
                if ui.input(|input| input.modifiers.shift) {
                    self.previous_search_match();
                } else {
                    self.execute_search();
                }
            }

            if ui
                .button(t!("textviewer.search_button").to_string())
                .clicked()
            {
                self.execute_search();
            }

            let has_results = self.search.occurrences.len() > 0;
            let status = if self.search.submitted_query.is_empty() {
                String::new()
            } else if has_results {
                t!(
                    "textviewer.search_result_count",
                    current = (self.search.current + 1).to_string(),
                    total = self.search.occurrences.len().to_string()
                )
                .to_string()
            } else {
                t!("textviewer.search_no_results").to_string()
            };
            ui.label(status);

            ui.add_enabled_ui(has_results, |ui| {
                if ui
                    .button(t!("textviewer.search_prev_button").to_string())
                    .on_hover_text(t!("textviewer.search_prev").to_string())
                    .clicked()
                {
                    self.previous_search_match();
                }
                if ui
                    .button(t!("textviewer.search_next_button").to_string())
                    .on_hover_text(t!("textviewer.search_next").to_string())
                    .clicked()
                {
                    self.next_search_match();
                }
            });

            if ui
                .button(t!("textviewer.search_close_button").to_string())
                .on_hover_text(t!("textviewer.search_close").to_string())
                .clicked()
            {
                self.close_search();
            }
        });
    }

    pub(super) fn show_searchable_line(&self, ui: &mut egui::Ui, line_index: usize, line: &str) {
        let line_start = self.line_offsets[line_index];
        let first = self.search.occurrences.lower_bound(line_start);
        let last = self
            .search
            .occurrences
            .lower_bound(line_start + line.len() as u32);
        let label = if first == last {
            egui::Label::new(egui::RichText::new(line).monospace().size(self.font_size))
                .selectable(true)
        } else {
            egui::Label::new(highlighted_line_job(
                LineMatchSearch {
                    line,
                    line_start,
                    occurrences: &self.search.occurrences,
                    query: &self.search.normalized_query,
                    match_range: first..last,
                    current: self.search.current,
                },
                self.font_size,
                ui.visuals().text_color(),
            ))
            .selectable(true)
        };
        if self.word_wrap {
            ui.add(label.wrap());
        } else {
            ui.add(label);
        }
    }
}

fn highlighted_line_job(
    search: LineMatchSearch<'_>,
    font_size: f32,
    text_color: egui::Color32,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let mut pending_ends = BinaryHeap::<Reverse<(usize, bool)>>::new();
    let mut cursor = 0;
    let mut active_matches = 0usize;
    let mut active_current = 0usize;

    for match_index in search.match_range {
        let Some(offset) = search.occurrences.get(match_index) else {
            continue;
        };
        let Some(range) = matched_range(
            search.line,
            (offset - search.line_start) as usize,
            search.query,
        ) else {
            continue;
        };

        while let Some(Reverse((end, current))) = pending_ends.peek().copied() {
            if end > range.start {
                break;
            }
            pending_ends.pop();
            append_line_segment(
                &mut job,
                search.line,
                cursor..end,
                font_size,
                text_color,
                segment_highlight(active_matches, active_current),
            );
            cursor = cursor.max(end);
            active_matches -= 1;
            if current {
                active_current -= 1;
            }
        }

        append_line_segment(
            &mut job,
            search.line,
            cursor..range.start,
            font_size,
            text_color,
            segment_highlight(active_matches, active_current),
        );
        cursor = cursor.max(range.start);
        let current = match_index == search.current;
        active_matches += 1;
        if current {
            active_current += 1;
        }
        pending_ends.push(Reverse((range.end, current)));
    }

    while let Some(Reverse((end, current))) = pending_ends.pop() {
        append_line_segment(
            &mut job,
            search.line,
            cursor..end,
            font_size,
            text_color,
            segment_highlight(active_matches, active_current),
        );
        cursor = cursor.max(end);
        active_matches -= 1;
        if current {
            active_current -= 1;
        }
    }
    append_line_segment(
        &mut job,
        search.line,
        cursor..search.line.len(),
        font_size,
        text_color,
        segment_highlight(active_matches, active_current),
    );
    job
}

#[derive(Clone, Copy)]
enum SegmentHighlight {
    None,
    Match,
    Current,
}

fn segment_highlight(matches: usize, current: usize) -> SegmentHighlight {
    if current > 0 {
        SegmentHighlight::Current
    } else if matches > 0 {
        SegmentHighlight::Match
    } else {
        SegmentHighlight::None
    }
}

fn append_line_segment(
    job: &mut egui::text::LayoutJob,
    line: &str,
    range: Range<usize>,
    font_size: f32,
    text_color: egui::Color32,
    highlight: SegmentHighlight,
) {
    if range.start >= range.end {
        return;
    }
    let mut format = egui::TextFormat::simple(egui::FontId::monospace(font_size), text_color);
    format.background = match highlight {
        SegmentHighlight::None => egui::Color32::TRANSPARENT,
        SegmentHighlight::Match => egui::Color32::from_rgba_unmultiplied(70, 170, 255, 48),
        SegmentHighlight::Current => egui::Color32::from_rgba_unmultiplied(255, 190, 40, 72),
    };
    let previous_sections = job.sections.len();
    let previous_text_len = job.text.len();
    job.append(&line[range], 0.0, format);
    if previous_sections > 0 && job.sections.len() == previous_sections + 1 {
        let previous = &job.sections[previous_sections - 1];
        let current = &job.sections[previous_sections];
        if previous.byte_range.end == egui::text::ByteIndex(previous_text_len)
            && previous.format.background == current.format.background
        {
            let end = current.byte_range.end;
            job.sections.pop();
            job.sections[previous_sections - 1].byte_range.end = end;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_overlapping_occurrences_compactly() {
        let offsets = super::super::build_line_offsets("banana");
        let (occurrences, _) = find_occurrences("banana", &offsets, "ana");

        assert_eq!(occurrences.len(), 2);
        assert_eq!(occurrences.get(0), Some(1));
        assert_eq!(occurrences.get(1), Some(3));
        assert!(occurrences.deltas.len() < occurrences.len() * 4);
    }

    #[test]
    fn maps_case_insensitive_unicode_matches_to_original_utf8_ranges() {
        let content = "猫Café\n";
        let offsets = super::super::build_line_offsets(content);
        let (occurrences, normalized_query) = find_occurrences(content, &offsets, "CAFÉ");

        assert_eq!(occurrences.get(0), Some(3));
        assert_eq!(matched_range("猫Café", 3, &normalized_query), Some(3..8));
    }

    #[test]
    fn matches_dotted_i_without_returning_an_invalid_utf8_range() {
        let content = "İ";
        let offsets = super::super::build_line_offsets(content);
        let (occurrences, normalized_query) = find_occurrences(content, &offsets, "i");

        assert_eq!(occurrences.get(0), Some(0));
        assert_eq!(matched_range(content, 0, &normalized_query), Some(0..2));
    }

    #[test]
    fn occurrence_checkpoints_support_random_access() {
        let mut occurrences = OccurrenceIndex::default();
        for offset in 0..1024u32 {
            occurrences.push(offset * 2);
        }

        assert_eq!(occurrences.len(), 1024);
        assert_eq!(occurrences.get(0), Some(0));
        assert_eq!(occurrences.get(255), Some(510));
        assert_eq!(occurrences.get(256), Some(512));
        assert_eq!(occurrences.get(1023), Some(2046));
        assert_eq!(occurrences.lower_bound(513), 257);
    }

    #[test]
    fn search_state_submits_once_and_wraps_navigation() {
        let content = "banana";
        let offsets = super::super::build_line_offsets(content);
        let mut state = TextSearchState {
            query: "ana".to_owned(),
            ..Default::default()
        };

        assert_eq!(state.occurrences.len(), 0);
        assert_eq!(state.submit(content, &offsets), Some(1));
        assert_eq!(state.next_offset(), Some(3));
        assert_eq!(state.next_offset(), Some(1));
        assert_eq!(state.previous_offset(), Some(3));
    }

    #[test]
    fn searches_each_line_independently() {
        let content = "sea\nurch";
        let offsets = super::super::build_line_offsets(content);
        let (occurrences, _) = find_occurrences(content, &offsets, "search");

        assert_eq!(occurrences.len(), 0);
    }

    #[test]
    fn highlighted_ranges_prioritize_the_current_overlapping_match() {
        let mut occurrences = OccurrenceIndex::default();
        occurrences.push(1);
        occurrences.push(3);
        let query = "ana".chars().collect::<Vec<_>>();
        let job = highlighted_line_job(
            LineMatchSearch {
                line: "banana",
                line_start: 0,
                occurrences: &occurrences,
                query: &query,
                match_range: 0..2,
                current: 1,
            },
            14.0,
            egui::Color32::WHITE,
        );

        assert_eq!(job.text, "banana");
        assert_eq!(job.sections.len(), 3);
        assert_eq!(
            job.sections[0].format.background,
            egui::Color32::TRANSPARENT
        );
        assert_eq!(
            job.sections[1].format.background,
            egui::Color32::from_rgba_unmultiplied(70, 170, 255, 48)
        );
        assert_eq!(
            job.sections[2].format.background,
            egui::Color32::from_rgba_unmultiplied(255, 190, 40, 72)
        );
    }

    #[test]
    fn coalesces_dense_match_highlights() {
        let mut occurrences = OccurrenceIndex::default();
        for offset in 0..4 {
            occurrences.push(offset);
        }
        let query = vec!['a'];
        let job = highlighted_line_job(
            LineMatchSearch {
                line: "aaaa",
                line_start: 0,
                occurrences: &occurrences,
                query: &query,
                match_range: 0..4,
                current: 2,
            },
            14.0,
            egui::Color32::WHITE,
        );

        assert_eq!(job.sections.len(), 3);
        assert_eq!(job.text, "aaaa");
    }

    #[test]
    fn view_can_keep_search_state_separate_from_query_editing() {
        let mut state = TextSearchState {
            query: "needle".to_owned(),
            ..Default::default()
        };
        state.clear_results();

        assert_eq!(state.query, "needle");
        assert!(state.submitted_query.is_empty());
        assert_eq!(state.occurrences.len(), 0);
    }
}
