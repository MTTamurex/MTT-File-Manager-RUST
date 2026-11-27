use super::TextViewerApp;
use eframe::egui;
use pulldown_cmark::{Event, Options, Parser, Tag};
use std::ops::Range;

impl TextViewerApp {
    pub(super) fn show_markdown_content(&mut self, ui: &mut egui::Ui) {
        let scroll_id = egui::Id::new("text_viewer_markdown").with(&self.file_path);
        let font_scale = self.font_size / super::DEFAULT_FONT_SIZE;
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
            let style = ui.style_mut();
            for font_id in style.text_styles.values_mut() {
                font_id.size *= font_scale;
            }
            style.wrap_mode = Some(egui::TextWrapMode::Wrap);
            style.url_in_tooltip = true;
            self.markdown_document
                .show(ui, &mut self.markdown_cache, &self.content, scroll_id);
        });
    }
}

#[derive(Default)]
pub(super) struct MarkdownDocument {
    blocks: Vec<Range<usize>>,
    references: String,
    offsets: Vec<f32>,
    layout: Option<LayoutKey>,
    width: f32,
    #[cfg(test)]
    pub(super) rendered_blocks: Range<usize>,
}

#[derive(PartialEq)]
struct LayoutKey {
    width: f32,
    pixels_per_point: f32,
    text_styles: std::collections::BTreeMap<egui::TextStyle, egui::FontId>,
}

impl MarkdownDocument {
    pub(super) fn invalidate_layout(&mut self) {
        self.layout = None;
    }

    pub(super) fn new(source: &str) -> Self {
        let parser = Parser::new_ext(source, parser_options());
        let mut references = String::new();
        for (_, definition) in parser.reference_definitions().iter() {
            references.push('\n');
            references.push_str(&source[definition.span.clone()]);
            references.push('\n');
        }
        let mut blocks = Vec::new();
        let mut depth = 0usize;
        let mut start = 0;
        let mut has_footnotes = false;
        for (event, range) in parser.into_offset_iter() {
            match event {
                Event::Start(tag) => {
                    has_footnotes |= matches!(tag, Tag::FootnoteDefinition(_));
                    if depth == 0 {
                        start = range.start;
                    }
                    depth += 1;
                }
                Event::End(_) => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        blocks.push(start..range.end);
                    }
                }
                _ if depth == 0 => blocks.push(range),
                _ => {}
            }
        }
        // Footnote references need document-wide parser context. Keep these
        // documents intact rather than silently turning references into plain text.
        if has_footnotes {
            blocks.clear();
            blocks.push(0..source.len());
            references.clear();
        }
        Self {
            blocks,
            references,
            ..Default::default()
        }
    }

    fn show(
        &mut self,
        ui: &mut egui::Ui,
        cache: &mut egui_commonmark::CommonMarkCache,
        source: &str,
        id: egui::Id,
    ) -> egui::Id {
        egui::ScrollArea::both()
            .id_salt(id)
            .auto_shrink([false, false])
            .show_viewport(ui, |ui, viewport| {
                let width = ui.available_width().max(1.0);
                let layout = LayoutKey {
                    width,
                    pixels_per_point: ui.ctx().pixels_per_point(),
                    text_styles: ui.style().text_styles.clone(),
                };
                let measure = self.layout.as_ref() != Some(&layout);
                let visible = if measure {
                    self.offsets.clear();
                    self.offsets.push(0.0);
                    self.width = width;
                    0..self.blocks.len()
                } else {
                    self.visible_blocks(viewport)
                };
                #[cfg(test)]
                {
                    self.rendered_blocks = visible.clone();
                }
                let origin = ui.cursor().min.y;
                if !measure {
                    ui.add_space(self.offsets[visible.start]);
                }
                for index in visible {
                    ui.push_id(index, |ui| {
                        let block = &source[self.blocks[index].clone()];
                        let with_references;
                        let markdown = if self.references.is_empty() {
                            block
                        } else {
                            with_references = format!("{block}\n\n{}", self.references);
                            &with_references
                        };
                        crate::ui::preview_panel::markdown_preview::show_markdown(
                            ui,
                            cache,
                            markdown,
                            Some(width as usize),
                        );
                    });
                    if measure {
                        self.offsets.push(ui.cursor().min.y - origin);
                        self.width = self.width.max(ui.min_rect().width());
                    }
                }
                let height = self.offsets.last().copied().unwrap_or(0.0);
                ui.set_min_size(egui::vec2(self.width, height));
                self.layout = Some(layout);
            })
            .id
    }

    fn visible_blocks(&self, viewport: egui::Rect) -> Range<usize> {
        let start = self
            .offsets
            .partition_point(|offset| *offset < viewport.min.y)
            .saturating_sub(1)
            .min(self.blocks.len());
        let end = self
            .offsets
            .partition_point(|offset| *offset <= viewport.max.y)
            .min(self.blocks.len());
        start..end.max(start)
    }
}

fn parser_options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_DEFINITION_LIST
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_complete_nested_blocks_and_reference_definitions() {
        let source = "# Title\n\n- first\n  - nested **text**\n- second\n\n```text\n# not a heading\n\nline\n```\n\n[reference][target]\n\n[target]: https://example.org \"Title\"\n";
        let document = MarkdownDocument::new(source);
        assert_eq!(document.blocks.len(), 4);
        assert!(source[document.blocks[1].clone()].contains("- second"));
        assert!(source[document.blocks[2].clone()].contains("# not a heading"));
        assert!(document
            .references
            .contains("[target]: https://example.org"));
    }

    #[test]
    fn footnotes_retain_document_wide_context() {
        let source = "# Title\n\nA note[^id].\n\n[^id]: Definition\n";
        let document = MarkdownDocument::new(source);
        assert_eq!(document.blocks, vec![0..source.len()]);
    }

    #[test]
    fn scrolls_to_distant_blocks_without_rendering_the_entire_document() {
        let license = "Example license terms for a synthetic dependency component.\n".repeat(80);
        let source =
            format!("## Heading\n\nParagraph with **bold** text.\n\n```\n{license}```\n\n")
                .repeat(132);
        let mut document = MarkdownDocument::new(&source);
        let mut cache = egui_commonmark::CommonMarkCache::default();
        let context = egui::Context::default();
        let source_id = egui::Id::new("synthetic_markdown");
        let input = || egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            ..Default::default()
        };
        let mut scroll_id = source_id;
        let _ = context.run_ui(input(), |ui| {
            scroll_id = document.show(ui, &mut cache, &source, source_id);
        });
        let total = document.blocks.len();
        assert_eq!(document.rendered_blocks, 0..total);
        let _ = context.run_ui(input(), |ui| {
            document.show(ui, &mut cache, &source, source_id);
        });
        assert!(document.rendered_blocks.len() < total / 10);
        let mut state = egui::scroll_area::State::load(&context, scroll_id).unwrap();
        state.offset.y = document.offsets[total / 2];
        state.store(&context, scroll_id);
        let _ = context.run_ui(input(), |ui| {
            document.show(ui, &mut cache, &source, source_id);
        });
        assert!(document.rendered_blocks.start >= total / 2 - 2);
        assert!(document.rendered_blocks.len() < total / 10);
        let _ = context.run_ui(input(), |ui| {
            ui.set_width(500.0);
            document.show(ui, &mut cache, &source, source_id);
        });
        assert_eq!(document.rendered_blocks, 0..total);
        let _ = context.run_ui(input(), |ui| {
            ui.style_mut()
                .text_styles
                .get_mut(&egui::TextStyle::Body)
                .unwrap()
                .size = 22.0;
            document.show(ui, &mut cache, &source, source_id);
        });
        assert_eq!(document.rendered_blocks, 0..total);
    }
}
