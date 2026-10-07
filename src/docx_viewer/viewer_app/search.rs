use eframe::egui;
use rust_i18n::t;

use super::super::search::{SearchBounds, SearchRequest, SearchResult};
use super::DocxViewerApp;

impl DocxViewerApp {
    pub(super) fn toggle_search(&mut self) {
        if self.search_active {
            self.close_search();
        } else {
            self.open_search();
        }
    }

    fn open_search(&mut self) {
        self.search_active = true;
        self.search_input_focus_requested = true;
    }

    fn close_search(&mut self) {
        self.search_active = false;
        self.search_input_has_focus = false;
        self.search_generation = self.search_generation.wrapping_add(1);
        if self.search_in_progress {
            self.cancel_search();
        }
        self.search_in_progress = false;
        self.search_results.clear();
        self.search_query.clear();
        self.current_match_idx = 0;
        self.last_searched_query.clear();
        self.scroll_to_match = None;
    }

    fn cancel_search(&self) {
        if let Some(worker) = &self.worker {
            worker.request_search(SearchRequest {
                query: String::new(),
                generation: self.search_generation,
            });
        }
    }

    fn execute_search(&mut self) {
        let query = self.search_query.trim().to_owned();
        if query.is_empty() {
            self.search_generation = self.search_generation.wrapping_add(1);
            if self.search_in_progress {
                self.cancel_search();
            }
            self.search_in_progress = false;
            self.search_results.clear();
            self.current_match_idx = 0;
            self.last_searched_query.clear();
            return;
        }
        if query == self.last_searched_query
            && (self.search_in_progress || !self.search_results.is_empty())
        {
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

    pub(super) fn accept_search_result(&mut self, result: SearchResult) {
        if !self.search_active
            || result.generation != self.search_generation
            || result.query != self.last_searched_query
            || result.query != self.search_query.trim()
        {
            return;
        }

        self.search_in_progress = false;
        self.search_results = result.matches;
        self.current_match_idx = 0;
        self.scroll_to_match = None;
        if !self.search_results.is_empty() {
            self.go_to_match(0);
        }
    }

    fn next_match(&mut self) {
        if self.search_results.is_empty() {
            return;
        }
        let next = (self.current_match_idx + 1) % self.search_results.len();
        self.go_to_match(next);
    }

    fn previous_match(&mut self) {
        if self.search_results.is_empty() {
            return;
        }
        let previous = if self.current_match_idx == 0 {
            self.search_results.len() - 1
        } else {
            self.current_match_idx - 1
        };
        self.go_to_match(previous);
    }

    fn go_to_match(&mut self, index: usize) {
        let Some(search_match) = self.search_results.get(index).copied() else {
            return;
        };
        self.current_match_idx = index;
        self.current_page = search_match.page_index;
        self.scroll_to_page = None;
        let fraction = search_match
            .bounds
            .and_then(|bounds| {
                self.page_sizes
                    .get(search_match.page_index)
                    .map(|(_, height)| (bounds.top / *height).clamp(0.0, 1.0))
            })
            .unwrap_or(0.0);
        self.scroll_to_match = Some((search_match.page_index, fraction));
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

        if self.search_active
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

        if self.search_active
            && ctx.input(|input| input.key_pressed(egui::Key::F3) && !input.modifiers.ctrl)
        {
            if ctx.input(|input| input.modifiers.shift) {
                self.previous_match();
            } else {
                self.next_match();
            }
        }
    }

    pub(super) fn show_search_bar(&mut self, root_ui: &mut egui::Ui) {
        if !self.search_active {
            return;
        }

        let (bar_fill, separator) =
            crate::ui::theme::viewer_bar_colors(root_ui.visuals().dark_mode);
        egui::Panel::top("docx_search_bar")
            .frame(
                egui::Frame::new()
                    .fill(bar_fill)
                    .inner_margin(egui::Margin::symmetric(10, 6)),
            )
            .show(root_ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.search_query)
                            .hint_text(t!("docxviewer.search_placeholder").to_string())
                            .desired_width(240.0),
                    );
                    self.search_input_has_focus = response.has_focus();

                    if response.changed() && self.search_query.trim() != self.last_searched_query {
                        self.search_generation = self.search_generation.wrapping_add(1);
                        if self.search_in_progress {
                            self.cancel_search();
                        }
                        self.search_in_progress = false;
                        self.search_results.clear();
                        self.current_match_idx = 0;
                        self.last_searched_query.clear();
                        self.scroll_to_match = None;
                    }

                    if self.search_input_focus_requested {
                        response.request_focus();
                        self.search_input_focus_requested = false;
                    }

                    let enter_pressed = ui.input(|input| input.key_pressed(egui::Key::Enter));
                    if enter_pressed && (response.has_focus() || response.lost_focus()) {
                        if ui.input(|input| input.modifiers.shift) {
                            self.previous_match();
                        } else {
                            self.execute_search();
                        }
                    }

                    if ui
                        .button(t!("docxviewer.search_button").to_string())
                        .clicked()
                    {
                        self.execute_search();
                    }

                    let status = if self.search_in_progress {
                        t!("docxviewer.search_searching").to_string()
                    } else if !self.last_searched_query.is_empty() && self.search_results.is_empty()
                    {
                        t!("docxviewer.search_no_results").to_string()
                    } else if !self.search_results.is_empty() {
                        t!(
                            "docxviewer.search_result_count",
                            current = (self.current_match_idx + 1).to_string(),
                            total = self.search_results.len().to_string()
                        )
                        .to_string()
                    } else {
                        String::new()
                    };
                    ui.label(status);

                    let has_results = !self.search_results.is_empty();
                    ui.add_enabled_ui(has_results, |ui| {
                        if ui
                            .button(t!("docxviewer.search_prev_button").to_string())
                            .on_hover_text(t!("docxviewer.search_prev").to_string())
                            .clicked()
                        {
                            self.previous_match();
                        }
                        if ui
                            .button(t!("docxviewer.search_next_button").to_string())
                            .on_hover_text(t!("docxviewer.search_next").to_string())
                            .clicked()
                        {
                            self.next_match();
                        }
                    });

                    if ui
                        .button(t!("docxviewer.search_close_button").to_string())
                        .on_hover_text(t!("docxviewer.search_close").to_string())
                        .clicked()
                    {
                        self.close_search();
                    }
                });

                let inner = ui.max_rect();
                ui.painter().hline(
                    inner.x_range(),
                    inner.max.y + 6.0,
                    egui::Stroke::new(1.0, separator),
                );
            });
    }

    pub(super) fn paint_search_highlights(
        &self,
        painter: &egui::Painter,
        page_index: usize,
        page_rect: egui::Rect,
        page_scale: f32,
    ) {
        for (index, search_match) in self.search_results.iter().enumerate() {
            if search_match.page_index != page_index {
                continue;
            }
            let Some(bounds) = search_match.bounds else {
                continue;
            };
            let Some(rect) = search_bounds_to_rect(bounds, page_rect, page_scale) else {
                continue;
            };
            let (fill, stroke, underline) = if index == self.current_match_idx {
                (
                    egui::Color32::from_rgba_unmultiplied(255, 190, 40, 48),
                    egui::Stroke::new(1.5, egui::Color32::from_rgba_unmultiplied(230, 105, 0, 230)),
                    egui::Stroke::new(2.0, egui::Color32::from_rgba_unmultiplied(230, 105, 0, 255)),
                )
            } else {
                (
                    egui::Color32::from_rgba_unmultiplied(70, 170, 255, 34),
                    egui::Stroke::new(
                        0.8,
                        egui::Color32::from_rgba_unmultiplied(20, 120, 210, 170),
                    ),
                    egui::Stroke::new(
                        1.2,
                        egui::Color32::from_rgba_unmultiplied(20, 120, 210, 210),
                    ),
                )
            };
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
        }
    }
}

fn search_bounds_to_rect(
    bounds: SearchBounds,
    page_rect: egui::Rect,
    page_scale: f32,
) -> Option<egui::Rect> {
    let rect = egui::Rect::from_min_max(
        page_rect.min + egui::vec2(bounds.left * page_scale, bounds.top * page_scale),
        page_rect.min + egui::vec2(bounds.right * page_scale, bounds.bottom * page_scale),
    )
    .intersect(page_rect);
    (rect.width() > 0.0 && rect.height() > 0.0).then_some(rect)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_natural_page_bounds_using_display_zoom() {
        let page_rect = egui::Rect::from_min_size(egui::pos2(20.0, 30.0), egui::vec2(200.0, 300.0));
        let bounds = SearchBounds {
            left: 10.0,
            top: 20.0,
            right: 30.0,
            bottom: 40.0,
        };

        let rect = search_bounds_to_rect(bounds, page_rect, 2.0).unwrap();

        assert_eq!(rect.min, egui::pos2(40.0, 70.0));
        assert_eq!(rect.max, egui::pos2(80.0, 110.0));
    }

    #[test]
    fn clips_highlights_to_page_edges() {
        let page_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(100.0, 100.0));
        let bounds = SearchBounds {
            left: -10.0,
            top: -5.0,
            right: 20.0,
            bottom: 10.0,
        };

        let rect = search_bounds_to_rect(bounds, page_rect, 1.0).unwrap();

        assert_eq!(rect.min, egui::Pos2::ZERO);
        assert_eq!(rect.max, egui::pos2(20.0, 10.0));
    }

    #[test]
    fn discards_search_results_from_older_generations() {
        let mut app = DocxViewerApp::new(std::path::PathBuf::new(), false);
        app.search_active = true;
        app.search_query = "needle".to_owned();
        app.last_searched_query = "needle".to_owned();
        app.search_generation = 2;
        app.search_in_progress = true;

        app.accept_search_result(SearchResult {
            query: "needle".to_owned(),
            generation: 1,
            matches: Vec::new(),
        });

        assert!(app.search_in_progress);
        assert!(app.search_results.is_empty());
    }
}
