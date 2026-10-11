use clipboard_win::{formats, Clipboard, Setter};
use eframe::egui;
use rust_i18n::t;

use super::super::search::SearchBounds;
use super::DocxViewerApp;

pub(super) struct DragSelection {
    page_index: usize,
    anchor: egui::Pos2,
    current: egui::Pos2,
}

pub(super) struct TextSelection {
    pub(super) page_index: usize,
    pub(super) bounds: SearchBounds,
    text: String,
    generation: u64,
}

impl DocxViewerApp {
    pub(super) fn handle_page_selection(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        page_index: usize,
        page_rect: egui::Rect,
        page_scale: f32,
    ) {
        if response.drag_started() {
            ui.ctx().memory_mut(|memory| memory.stop_text_input());
            self.search_input_has_focus = false;
            if let Some(point) = response
                .interact_pointer_pos()
                .and_then(|pos| screen_to_page(pos, page_rect, page_scale))
            {
                self.selection_generation = self.selection_generation.wrapping_add(1);
                self.selection = None;
                self.drag_selection = Some(DragSelection {
                    page_index,
                    anchor: point,
                    current: point,
                });
            }
        }

        let pointer_point = response
            .interact_pointer_pos()
            .and_then(|pos| screen_to_page(pos, page_rect, page_scale));
        if let Some(drag) = self.drag_selection.as_mut() {
            if drag.page_index == page_index {
                if let Some(point) = pointer_point {
                    drag.current = point;
                }
                if response.drag_stopped() {
                    let drag = self.drag_selection.take().expect("active DOCX selection");
                    if let Some(bounds) = selection_bounds(drag.anchor, drag.current) {
                        self.request_text_selection(page_index, bounds);
                    }
                }
            }
        }

        if response.clicked() && !response.dragged() {
            ui.ctx().memory_mut(|memory| memory.stop_text_input());
            self.search_input_has_focus = false;
            self.selection_generation = self.selection_generation.wrapping_add(1);
            self.drag_selection = None;
            self.selection = None;
        }

        self.paint_selection_overlay(ui.painter(), page_index, page_rect, page_scale);
    }

    fn request_text_selection(&mut self, page_index: usize, bounds: SearchBounds) {
        let generation = self.selection_generation;
        let requested = self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.request_text_selection(page_index, bounds, generation));
        if requested {
            self.selection = Some(TextSelection {
                page_index,
                bounds,
                text: String::new(),
                generation,
            });
        }
    }

    pub(super) fn receive_selected_text(
        &mut self,
        page_index: usize,
        generation: u64,
        text: String,
    ) {
        if generation != self.selection_generation {
            return;
        }
        let Some(selection) = self.selection.as_mut() else {
            return;
        };
        if selection.page_index != page_index || selection.generation != generation {
            return;
        }
        selection.text = text.replace("\r\n", "\n").trim().to_owned();
        if selection.text.is_empty() {
            self.selection = None;
        }
    }

    pub(super) fn handle_selection_shortcuts(&mut self, ctx: &egui::Context) {
        if !self.has_text_selection() || self.search_input_has_focus {
            return;
        }
        let should_copy = ctx.input_mut(|input| {
            if let Some(index) = input
                .events
                .iter()
                .position(|event| matches!(event, egui::Event::Copy))
            {
                input.events.remove(index);
                true
            } else {
                false
            }
        });
        if should_copy {
            if let Err(error) = self.copy_selected_text() {
                log::warn!("[DOCX-VIEWER] copy selection failed: {error}");
            }
        }
    }

    pub(super) fn has_text_selection(&self) -> bool {
        self.selection
            .as_ref()
            .is_some_and(|selection| !selection.text.is_empty())
    }

    pub(super) fn copy_selected_text(&self) -> Result<(), String> {
        let text = self
            .selection
            .as_ref()
            .filter(|selection| !selection.text.is_empty())
            .map(|selection| &selection.text)
            .ok_or_else(|| t!("docxviewer.selection_missing").to_string())?;
        if let Ok(_clipboard) = Clipboard::new_attempts(3) {
            if formats::Unicode.write_clipboard(text).is_ok() {
                return Ok(());
            }
        }
        Err(t!("operations.error_clipboard").to_string())
    }

    pub(super) fn selection_summary(&self) -> String {
        match self.selection.as_ref() {
            Some(selection) if !selection.text.is_empty() => t!(
                "docxviewer.selection_ready",
                count = selection.text.chars().count()
            )
            .to_string(),
            _ => t!("docxviewer.selection_hint").to_string(),
        }
    }

    fn paint_selection_overlay(
        &self,
        painter: &egui::Painter,
        page_index: usize,
        page_rect: egui::Rect,
        page_scale: f32,
    ) {
        let bounds = self
            .selection
            .as_ref()
            .filter(|selection| selection.page_index == page_index)
            .map(|selection| selection.bounds)
            .or_else(|| {
                self.drag_selection
                    .as_ref()
                    .filter(|selection| selection.page_index == page_index)
                    .and_then(|selection| selection_bounds(selection.anchor, selection.current))
            });
        let Some(bounds) = bounds else {
            return;
        };
        let rect = egui::Rect::from_min_max(
            page_rect.min + egui::vec2(bounds.left * page_scale, bounds.top * page_scale),
            page_rect.min + egui::vec2(bounds.right * page_scale, bounds.bottom * page_scale),
        );
        painter.rect_filled(
            rect,
            1.0,
            egui::Color32::from_rgba_unmultiplied(88, 156, 255, 58),
        );
        painter.rect(
            rect,
            1.0,
            egui::Color32::TRANSPARENT,
            egui::Stroke::new(1.0, egui::Color32::from_rgb(88, 156, 255)),
            egui::StrokeKind::Outside,
        );
    }
}

fn screen_to_page(pos: egui::Pos2, rect: egui::Rect, scale: f32) -> Option<egui::Pos2> {
    (scale.is_finite() && scale > 0.0 && rect.contains(pos))
        .then(|| egui::pos2((pos.x - rect.left()) / scale, (pos.y - rect.top()) / scale))
}

fn selection_bounds(start: egui::Pos2, end: egui::Pos2) -> Option<SearchBounds> {
    let left = start.x.min(end.x);
    let right = start.x.max(end.x);
    let top = start.y.min(end.y);
    let bottom = start.y.max(end.y);
    let center_x = (left + right) * 0.5;
    let center_y = (top + bottom) * 0.5;
    SearchBounds::new(
        center_x - (right - left).max(2.0) * 0.5,
        center_y - (bottom - top).max(2.0) * 0.5,
        center_x + (right - left).max(2.0) * 0.5,
        center_y + (bottom - top).max(2.0) * 0.5,
    )
}
