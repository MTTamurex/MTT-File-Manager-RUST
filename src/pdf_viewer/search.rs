//! PDF full-text search: Ctrl+F to open, type query, Enter to run, F3/Shift+F3
//! to navigate between matches. Matches are highlighted on the page and the
//! current match has a stronger color. All search work happens on the search
//! worker thread; this module owns the UI state and the per-frame wiring.

use eframe::egui;
use rust_i18n::t;

use super::render_worker::SearchRequest;
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
            self.search_results = result.matches;
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

        let all_fill = egui::Color32::from_rgba_unmultiplied(255, 230, 0, 90);
        let all_stroke =
            egui::Stroke::new(0.5, egui::Color32::from_rgba_unmultiplied(180, 150, 0, 200));
        let current_fill = egui::Color32::from_rgba_unmultiplied(255, 160, 0, 130);
        let current_stroke =
            egui::Stroke::new(1.5, egui::Color32::from_rgba_unmultiplied(220, 110, 0, 255));

        for (i, m) in self.search_results.iter().enumerate() {
            if m.page_idx != page_idx {
                continue;
            }
            let rect = self.page_bounds_to_screen_rect(page_idx, page_rect, m.bounds);
            if i == self.current_match_idx {
                painter.rect_filled(rect, 1.0, current_fill);
                painter.rect(
                    rect,
                    1.0,
                    egui::Color32::from_rgba_unmultiplied(255, 160, 0, 30),
                    current_stroke,
                    egui::StrokeKind::Outside,
                );
            } else {
                painter.rect_filled(rect, 1.0, all_fill);
                painter.rect(
                    rect,
                    1.0,
                    egui::Color32::from_rgba_unmultiplied(255, 230, 0, 20),
                    all_stroke,
                    egui::StrokeKind::Outside,
                );
            }
        }
    }
}
