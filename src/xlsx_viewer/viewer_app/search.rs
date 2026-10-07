use eframe::egui;
use rust_i18n::t;

use super::super::renderer::XlsxSearchMatch;
use super::XlsxViewerApp;

impl XlsxViewerApp {
    pub(super) fn toggle_search(&mut self) {
        if self.search_active {
            self.close_search();
        } else {
            self.search_active = true;
            self.search_input_focus_requested = true;
        }
    }

    fn close_search(&mut self) {
        self.search_active = false;
        self.search_input_has_focus = false;
        self.search_generation = self.search_generation.wrapping_add(1);
        self.search_in_progress = false;
        self.search_results.clear();
        self.search_query.clear();
        self.current_match_idx = 0;
        self.last_searched_query.clear();
        self.pending_search_navigation = None;
    }

    fn execute_search(&mut self) {
        let query = self.search_query.trim().to_owned();
        if query.is_empty() {
            self.search_generation = self.search_generation.wrapping_add(1);
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
        self.search_in_progress = self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.request_search(self.search_generation, query.clone()));
        self.last_searched_query = query;
        self.search_results.clear();
        self.current_match_idx = 0;
    }

    pub(super) fn accept_search_result(
        &mut self,
        generation: u64,
        query: String,
        matches: Vec<XlsxSearchMatch>,
    ) {
        if !self.search_active
            || generation != self.search_generation
            || query != self.last_searched_query
            || query != self.search_query.trim()
        {
            return;
        }

        self.search_in_progress = false;
        self.search_results = matches;
        self.current_match_idx = 0;
        if !self.search_results.is_empty() {
            self.go_to_match(0);
        }
    }

    fn next_match(&mut self) {
        if self.search_results.is_empty() {
            return;
        }
        self.go_to_match((self.current_match_idx + 1) % self.search_results.len());
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
        let Some(search_match) = self.search_results.get(index).cloned() else {
            return;
        };
        self.current_match_idx = index;
        self.select_sheet_for_search(&search_match);
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
        egui::Panel::top("xlsx_search_bar")
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
                            .hint_text(t!("xlsxviewer.search_placeholder").to_string())
                            .desired_width(240.0),
                    );
                    self.search_input_has_focus = response.has_focus();

                    if response.changed() && self.search_query.trim() != self.last_searched_query {
                        self.search_generation = self.search_generation.wrapping_add(1);
                        self.search_in_progress = false;
                        self.search_results.clear();
                        self.current_match_idx = 0;
                        self.last_searched_query.clear();
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
                        .button(t!("xlsxviewer.search_button").to_string())
                        .clicked()
                    {
                        self.execute_search();
                    }

                    let status = if self.search_in_progress {
                        t!("xlsxviewer.search_searching").to_string()
                    } else if !self.last_searched_query.is_empty() && self.search_results.is_empty()
                    {
                        t!("xlsxviewer.search_no_results").to_string()
                    } else if !self.search_results.is_empty() {
                        t!(
                            "xlsxviewer.search_result_count",
                            current = (self.current_match_idx + 1).to_string(),
                            total = self.search_results.len().to_string()
                        )
                        .to_string()
                    } else {
                        String::new()
                    };
                    ui.label(status);
                    if let Some(current_match) = self.search_results.get(self.current_match_idx) {
                        ui.add_sized(
                            [180.0, ui.spacing().interact_size.y],
                            egui::Label::new(&current_match.address).truncate(),
                        )
                        .on_hover_text(&current_match.text);
                    }

                    let has_results = !self.search_results.is_empty();
                    ui.add_enabled_ui(has_results, |ui| {
                        if ui
                            .button(t!("xlsxviewer.search_prev_button").to_string())
                            .on_hover_text(t!("xlsxviewer.search_prev").to_string())
                            .clicked()
                        {
                            self.previous_match();
                        }
                        if ui
                            .button(t!("xlsxviewer.search_next_button").to_string())
                            .on_hover_text(t!("xlsxviewer.search_next").to_string())
                            .clicked()
                        {
                            self.next_match();
                        }
                    });
                });
                let inner = ui.max_rect();
                ui.painter().hline(
                    inner.x_range(),
                    inner.max.y + 6.0,
                    egui::Stroke::new(1.0, separator),
                );
            });
    }
}
