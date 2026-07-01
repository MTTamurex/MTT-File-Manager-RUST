//! PDF full-text search: Ctrl+F to open, type query, Enter to run, F3/Shift+F3
//! to navigate between matches. Matches are highlighted on the page and the
//! current match has a stronger color. All search work happens on the search
//! worker thread; this module owns the UI state and the per-frame wiring.

use eframe::egui;
use rust_i18n::t;

use super::render_worker::{SearchMatch, SearchRequest};
use super::viewer_app::PdfViewerApp;

impl PdfViewerApp {
    pub(super) fn open_search(&mut self) {
        self.search_active = true;
        self.search_input_focus_requested = true;
        if self.search_generation == 0 {
            self.search_generation = 1;
        }
    }

    pub(super) fn close_search(&mut self) {
        self.search_active = false;
        let was_searching = self.search_in_progress;
        self.search_in_progress = false;
        self.search_generation = self.search_generation.wrapping_add(1);
        if was_searching {
            if let Some(worker) = &self.worker {
                worker.request_search(SearchRequest {
                    query: String::new(),
                    generation: self.search_generation,
                });
            }
        }
        self.search_results.clear();
        self.search_query.clear();
        self.current_match_idx = 0;
        self.last_searched_query.clear();
    }

    pub(super) fn toggle_search(&mut self) {
        if self.search_active {
            self.close_search();
        } else {
            self.open_search();
        }
    }

    pub(super) fn execute_search(&mut self) {
        let query = self.search_query.trim().to_string();
        if query.is_empty() {
            let was_searching = self.search_in_progress;
            self.search_generation = self.search_generation.wrapping_add(1);
            if was_searching {
                if let Some(worker) = &self.worker {
                    worker.request_search(SearchRequest {
                        query: String::new(),
                        generation: self.search_generation,
                    });
                }
            }
            self.search_results.clear();
            self.search_in_progress = false;
            self.last_searched_query.clear();
            return;
        }
        if query == self.last_searched_query && !self.search_results.is_empty() {
            return;
        }
        self.search_generation = self.search_generation.wrapping_add(1);
        self.search_in_progress = true;
        self.last_searched_query = query.clone();
        if let Some(worker) = &self.worker {
            worker.request_search(SearchRequest {
                query,
                generation: self.search_generation,
            });
        }
    }

    pub(super) fn poll_search_results(&mut self) {
        let Some(worker) = &self.worker else { return };
        for result in worker.drain_search_results() {
            let current_query = self.search_query.trim();
            if !self.search_active
                || result.generation != self.search_generation
                || result.query != self.last_searched_query
                || result.query != current_query
            {
                continue;
            }
            self.search_in_progress = false;
            self.search_results = dedup_search_matches(result.matches);
            if !self.search_results.is_empty() {
                self.current_match_idx = 0;
                let page_idx = self.search_results[0].page_idx;
                self.go_to_page(page_idx);
            } else {
                self.current_match_idx = 0;
            }
        }
    }

    pub(super) fn next_match(&mut self) {
        if self.search_results.is_empty() {
            return;
        }
        let next = (self.current_match_idx + 1) % self.search_results.len();
        self.go_to_match(next);
    }

    pub(super) fn prev_match(&mut self) {
        if self.search_results.is_empty() {
            return;
        }
        let prev = if self.current_match_idx == 0 {
            self.search_results.len() - 1
        } else {
            self.current_match_idx - 1
        };
        self.go_to_match(prev);
    }

    fn go_to_match(&mut self, idx: usize) {
        if self.search_results.is_empty() {
            return;
        }
        self.current_match_idx = idx;
        let page_idx = self.search_results[idx].page_idx;
        self.go_to_page(page_idx);
    }

    pub(super) fn handle_search_shortcuts(&mut self, ctx: &egui::Context) {
        let open_or_close = ctx.input_mut(|i| {
            if i.modifiers.ctrl && !i.modifiers.shift && i.key_pressed(egui::Key::F) {
                if let Some(pos) = i.events.iter().position(|e| {
                    matches!(
                        e,
                        egui::Event::Key {
                            key: egui::Key::F,
                            ..
                        }
                    )
                }) {
                    i.events.remove(pos);
                }
                true
            } else {
                false
            }
        });
        if open_or_close {
            self.toggle_search();
        }

        if self.search_active
            && ctx.input_mut(|i| {
                if i.key_pressed(egui::Key::Escape) {
                    if let Some(pos) = i.events.iter().position(|e| {
                        matches!(
                            e,
                            egui::Event::Key {
                                key: egui::Key::Escape,
                                ..
                            }
                        )
                    }) {
                        i.events.remove(pos);
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

        if self.search_active && ctx.input(|i| i.key_pressed(egui::Key::F3) && !i.modifiers.ctrl) {
            if ctx.input(|i| i.modifiers.shift) {
                self.prev_match();
            } else {
                self.next_match();
            }
        }
    }

    pub(super) fn show_search_bar(&mut self, ctx: &egui::Context) {
        if !self.search_active {
            return;
        }

        egui::TopBottomPanel::top("pdf_search_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;

                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.search_query)
                        .hint_text(t!("pdfviewer.search_placeholder").to_string())
                        .desired_width(240.0),
                );

                if response.changed() && self.search_query.trim() != self.last_searched_query {
                    let was_searching = self.search_in_progress;
                    self.search_generation = self.search_generation.wrapping_add(1);
                    if was_searching {
                        if let Some(worker) = &self.worker {
                            worker.request_search(SearchRequest {
                                query: String::new(),
                                generation: self.search_generation,
                            });
                        }
                    }
                    self.search_in_progress = false;
                    self.search_results.clear();
                    self.current_match_idx = 0;
                    self.last_searched_query.clear();
                }

                if self.search_input_focus_requested {
                    response.request_focus();
                    self.search_input_focus_requested = false;
                }

                let enter_pressed = ui.input(|i| i.key_pressed(egui::Key::Enter));
                if enter_pressed && (response.has_focus() || response.lost_focus()) {
                    if ui.input(|i| i.modifiers.shift) {
                        self.prev_match();
                    } else {
                        self.execute_search();
                    }
                }

                if ui
                    .button(t!("pdfviewer.search_button").to_string())
                    .clicked()
                {
                    self.execute_search();
                }

                let status = if self.search_in_progress {
                    t!("pdfviewer.search_searching").to_string()
                } else if !self.last_searched_query.is_empty() && self.search_results.is_empty() {
                    t!("pdfviewer.search_no_results").to_string()
                } else if !self.search_results.is_empty() {
                    t!(
                        "pdfviewer.search_result_count",
                        current = self.current_match_idx + 1,
                        total = self.search_results.len()
                    )
                    .to_string()
                } else {
                    String::new()
                };
                ui.label(status);

                let has_results = !self.search_results.is_empty();
                ui.add_enabled_ui(has_results, |ui| {
                    if ui
                        .button(t!("pdfviewer.search_prev_button").to_string())
                        .on_hover_text(t!("pdfviewer.search_prev").to_string())
                        .clicked()
                    {
                        self.prev_match();
                    }
                    if ui
                        .button(t!("pdfviewer.search_next_button").to_string())
                        .on_hover_text(t!("pdfviewer.search_next").to_string())
                        .clicked()
                    {
                        self.next_match();
                    }
                });

                if ui
                    .button(t!("pdfviewer.search_close_button").to_string())
                    .on_hover_text(t!("pdfviewer.search_close").to_string())
                    .clicked()
                {
                    self.close_search();
                }
            });
        });
    }

    pub(super) fn paint_search_highlights(
        &self,
        painter: &egui::Painter,
        page_idx: u32,
        page_rect: egui::Rect,
    ) {
        if self.search_results.is_empty() {
            return;
        }

        let all_fill = egui::Color32::from_rgba_unmultiplied(70, 170, 255, 34);
        let all_stroke = egui::Stroke::new(
            0.8,
            egui::Color32::from_rgba_unmultiplied(20, 120, 210, 170),
        );
        let all_underline = egui::Stroke::new(
            1.2,
            egui::Color32::from_rgba_unmultiplied(20, 120, 210, 210),
        );
        let current_fill = egui::Color32::from_rgba_unmultiplied(255, 190, 40, 48);
        let current_stroke =
            egui::Stroke::new(1.5, egui::Color32::from_rgba_unmultiplied(230, 105, 0, 230));
        let current_underline =
            egui::Stroke::new(2.0, egui::Color32::from_rgba_unmultiplied(230, 105, 0, 255));

        let paint_highlight = |rect: egui::Rect,
                               fill: egui::Color32,
                               stroke: egui::Stroke,
                               underline: egui::Stroke| {
            painter.rect_filled(rect, 1.5, fill);
            painter.rect(
                rect,
                1.5,
                egui::Color32::TRANSPARENT,
                stroke,
                egui::StrokeKind::Outside,
            );
            let y = (rect.bottom() - 1.0).max(rect.top());
            painter.line_segment(
                [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
                underline,
            );
        };

        let same_rect = |a: egui::Rect, b: egui::Rect| {
            (a.min.x - b.min.x).abs() < 0.5
                && (a.min.y - b.min.y).abs() < 0.5
                && (a.max.x - b.max.x).abs() < 0.5
                && (a.max.y - b.max.y).abs() < 0.5
        };

        let mut painted_rects = Vec::new();
        let mut current_rect = None;

        for (i, m) in self.search_results.iter().enumerate() {
            if m.page_idx != page_idx {
                continue;
            }
            let rect = self.page_bounds_to_screen_rect(page_idx, page_rect, m.bounds);
            if i == self.current_match_idx {
                current_rect = Some(rect);
            } else {
                if painted_rects
                    .iter()
                    .any(|painted| same_rect(*painted, rect))
                {
                    continue;
                }
                painted_rects.push(rect);
                paint_highlight(rect, all_fill, all_stroke, all_underline);
            }
        }

        if let Some(rect) = current_rect {
            paint_highlight(rect, current_fill, current_stroke, current_underline);
        }
    }
}

fn dedup_search_matches(matches: Vec<SearchMatch>) -> Vec<SearchMatch> {
    let mut unique = Vec::with_capacity(matches.len());

    for search_match in matches {
        if unique
            .iter()
            .any(|existing| same_search_target(existing, &search_match))
        {
            continue;
        }
        unique.push(search_match);
    }

    unique
}

fn same_search_target(a: &SearchMatch, b: &SearchMatch) -> bool {
    if a.page_idx != b.page_idx {
        return false;
    }

    let intersection = bounds_intersection_area(a.bounds, b.bounds);
    if intersection <= 0.0 {
        return false;
    }

    let smaller = bounds_area(a.bounds).min(bounds_area(b.bounds));
    smaller > 0.0 && intersection / smaller >= 0.55
}

fn bounds_area(bounds: super::renderer::PdfTextBounds) -> f32 {
    bounds.width().max(0.0) * bounds.height().max(0.0)
}

fn bounds_intersection_area(
    a: super::renderer::PdfTextBounds,
    b: super::renderer::PdfTextBounds,
) -> f32 {
    let left = a.left.max(b.left);
    let right = a.right.min(b.right);
    let bottom = a.bottom.max(b.bottom);
    let top = a.top.min(b.top);
    (right - left).max(0.0) * (top - bottom).max(0.0)
}
