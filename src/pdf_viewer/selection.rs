use clipboard_win::{formats, Clipboard, Setter};
use eframe::egui;
use rust_i18n::t;

use super::renderer::{PdfTextBounds, PdfTextSegment};
use super::viewer_app::PdfViewerApp;

#[derive(Clone, Copy)]
pub(super) struct DragSelection {
    pub page_idx: u32,
    anchor: PagePoint,
    current: PagePoint,
}

impl DragSelection {
    fn new(page_idx: u32, anchor: PagePoint) -> Self {
        Self {
            page_idx,
            anchor,
            current: anchor,
        }
    }

    fn update(&mut self, point: PagePoint) {
        self.current = point;
    }

    pub fn bounds(&self) -> PdfTextBounds {
        PdfTextBounds::from_points(
            self.anchor.x,
            self.current.x,
            self.anchor.y,
            self.current.y,
        )
    }
}

#[derive(Clone)]
pub(super) struct PageSelection {
    pub page_idx: u32,
    pub bounds: PdfTextBounds,
    pub text: String,
}

#[derive(Clone, Copy)]
struct PagePoint {
    x: f32,
    y: f32,
}

impl PdfViewerApp {
    pub(super) fn handle_selection_shortcuts(&mut self, ctx: &egui::Context) {
        if !self.has_text_selection() {
            return;
        }

        // egui-winit converts Ctrl+C into Event::Copy (never emits Event::Key for C),
        // so we must consume Event::Copy rather than using consume_shortcut.
        let should_copy = ctx.input_mut(|i| {
            if let Some(pos) = i.events.iter().position(|e| matches!(e, egui::Event::Copy)) {
                i.events.remove(pos);
                true
            } else {
                false
            }
        });

        if should_copy {
            if let Err(err) = self.copy_selected_text() {
                log::warn!("[PDF-VIEWER] copy selection failed: {err}");
            }
        }
    }

    pub(super) fn handle_page_selection(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        page_idx: u32,
        page_rect: egui::Rect,
    ) {
        if response.drag_started() {
            ui.ctx().memory_mut(|mem| mem.stop_text_input());

            if let Some(pointer_pos) = response.interact_pointer_pos() {
                if let Some(anchor) = self.screen_to_page_point(page_idx, page_rect, pointer_pos) {
                    self.drag_selection = Some(DragSelection::new(page_idx, anchor));
                    if self
                        .selection
                        .as_ref()
                        .map(|selection| selection.page_idx != page_idx)
                        .unwrap_or(false)
                    {
                        self.selection = None;
                    }
                    self.ensure_text_segments(page_idx);
                }
            }
        }

        let pointer_point = response
            .interact_pointer_pos()
            .and_then(|pointer_pos| self.screen_to_page_point(page_idx, page_rect, pointer_pos));

        if let Some(drag) = self.drag_selection.as_mut() {
            if drag.page_idx == page_idx {
                if let Some(point) = pointer_point {
                    drag.update(point);
                }

                if response.drag_stopped() {
                    let bounds = drag.bounds();
                    self.drag_selection = None;
                    self.finish_selection(page_idx, bounds);
                }
            }
        }

        if response.clicked() && !response.dragged() {
            ui.ctx().memory_mut(|mem| mem.stop_text_input());
            self.selection = None;
        }

        self.paint_selection_overlay(ui.painter(), page_idx, page_rect);
    }

    pub(super) fn has_text_selection(&self) -> bool {
        self.selection
            .as_ref()
            .map(|selection| !selection.text.is_empty())
            .unwrap_or(false)
    }

    pub(super) fn copy_selected_text(&self) -> Result<(), String> {
        let Some(selection) = self.selection.as_ref() else {
            return Err(t!("pdfviewer.selection_missing").to_string());
        };

        if selection.text.is_empty() {
            return Err(t!("pdfviewer.selection_missing").to_string());
        }

        if let Ok(_clip) = Clipboard::new_attempts(10) {
            if formats::Unicode.write_clipboard(&selection.text).is_ok() {
                return Ok(());
            }
        }

        Err(t!("operations.error_clipboard").to_string())
    }

    pub(super) fn selection_summary(&self) -> String {
        match self.selection.as_ref() {
            Some(selection) if !selection.text.is_empty() => {
                t!("pdfviewer.selection_ready", count = selection.text.chars().count()).to_string()
            }
            _ => t!("pdfviewer.selection_hint").to_string(),
        }
    }

    fn finish_selection(&mut self, page_idx: u32, bounds: PdfTextBounds) {
        let bounds = normalized_selection_bounds(bounds);

        let Some(segments) = self.ensure_text_segments(page_idx) else {
            self.selection = None;
            return;
        };

        let has_overlap = segments.iter().any(|segment| segment.bounds.overlaps(&bounds));

        if !has_overlap {
            self.selection = None;
            return;
        }

        let text = self
            .text_renderer
            .page_text_in_bounds(page_idx, bounds)
            .map(|text| normalize_selected_text(&text))
            .unwrap_or_else(|err| {
                log::warn!("[PDF-VIEWER] bounded text extraction failed: {err}");
                String::new()
            });

        if text.is_empty() {
            self.selection = None;
            return;
        }

        self.selection = Some(PageSelection {
            page_idx,
            bounds,
            text,
        });
    }

    fn ensure_text_segments(&mut self, page_idx: u32) -> Option<&Vec<PdfTextSegment>> {
        if !self.page_text.contains_key(&page_idx) {
            match self.text_renderer.page_text_segments(page_idx) {
                Ok(segments) => {
                    self.page_text.insert(page_idx, segments);
                }
                Err(err) => {
                    log::error!("[PDF-VIEWER] page {} text load failed: {err}", page_idx);
                    return None;
                }
            }
        }

        self.page_text.get(&page_idx)
    }

    fn paint_selection_overlay(
        &mut self,
        painter: &egui::Painter,
        page_idx: u32,
        page_rect: egui::Rect,
    ) {
        let highlight_fill = egui::Color32::from_rgba_unmultiplied(88, 156, 255, 72);
        let selection_fill = egui::Color32::from_rgba_unmultiplied(88, 156, 255, 24);
        let selection_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(88, 156, 255));

        if let Some(selection) = self.selection.clone() {
            if selection.page_idx == page_idx {
                let segments = self.ensure_text_segments(page_idx).cloned().unwrap_or_default();

                for segment in segments
                    .iter()
                    .filter(|segment| segment.bounds.overlaps(&selection.bounds))
                {
                    painter.rect_filled(
                        self.page_bounds_to_screen_rect(page_idx, page_rect, segment.bounds),
                        1.0,
                        highlight_fill,
                    );
                }

                painter.rect(
                    self.page_bounds_to_screen_rect(page_idx, page_rect, selection.bounds),
                    1.0,
                    selection_fill,
                    selection_stroke,
                    egui::StrokeKind::Outside,
                );
            }
        }

        if let Some(drag) = self.drag_selection {
            if drag.page_idx == page_idx {
                painter.rect(
                    self.page_bounds_to_screen_rect(page_idx, page_rect, drag.bounds()),
                    1.0,
                    selection_fill,
                    selection_stroke,
                    egui::StrokeKind::Outside,
                );
            }
        }
    }

    fn screen_to_page_point(
        &self,
        page_idx: u32,
        page_rect: egui::Rect,
        screen_pos: egui::Pos2,
    ) -> Option<PagePoint> {
        if page_rect.width() <= 0.0 || page_rect.height() <= 0.0 {
            return None;
        }

        let (natural_w, natural_h) = self.page_sizes[page_idx as usize];
        let (rotated_w, rotated_h) = if self.rotation % 180 != 0 {
            (natural_h, natural_w)
        } else {
            (natural_w, natural_h)
        };

        let u = ((screen_pos.x - page_rect.left()) / page_rect.width()).clamp(0.0, 1.0);
        let v = ((screen_pos.y - page_rect.top()) / page_rect.height()).clamp(0.0, 1.0);

        let rotated_x = u * rotated_w;
        let rotated_y = (1.0 - v) * rotated_h;

        let (x, y) = match self.rotation {
            90 => (natural_w - rotated_y, rotated_x),
            180 => (natural_w - rotated_x, natural_h - rotated_y),
            270 => (rotated_y, natural_h - rotated_x),
            _ => (rotated_x, rotated_y),
        };

        Some(PagePoint {
            x: x.clamp(0.0, natural_w),
            y: y.clamp(0.0, natural_h),
        })
    }

    fn page_bounds_to_screen_rect(
        &self,
        page_idx: u32,
        page_rect: egui::Rect,
        bounds: PdfTextBounds,
    ) -> egui::Rect {
        let corners = [
            self.page_point_to_screen(page_idx, page_rect, bounds.left, bounds.bottom),
            self.page_point_to_screen(page_idx, page_rect, bounds.left, bounds.top),
            self.page_point_to_screen(page_idx, page_rect, bounds.right, bounds.bottom),
            self.page_point_to_screen(page_idx, page_rect, bounds.right, bounds.top),
        ];

        let min_x = corners.iter().map(|point| point.x).fold(f32::INFINITY, f32::min);
        let max_x = corners
            .iter()
            .map(|point| point.x)
            .fold(f32::NEG_INFINITY, f32::max);
        let min_y = corners.iter().map(|point| point.y).fold(f32::INFINITY, f32::min);
        let max_y = corners
            .iter()
            .map(|point| point.y)
            .fold(f32::NEG_INFINITY, f32::max);

        egui::Rect::from_min_max(egui::pos2(min_x, min_y), egui::pos2(max_x, max_y))
    }

    fn page_point_to_screen(
        &self,
        page_idx: u32,
        page_rect: egui::Rect,
        x: f32,
        y: f32,
    ) -> egui::Pos2 {
        let (natural_w, natural_h) = self.page_sizes[page_idx as usize];
        let (rotated_x, rotated_y, rotated_w, rotated_h) = match self.rotation {
            90 => (y, natural_w - x, natural_h, natural_w),
            180 => (natural_w - x, natural_h - y, natural_w, natural_h),
            270 => (natural_h - y, x, natural_h, natural_w),
            _ => (x, y, natural_w, natural_h),
        };

        let u = if rotated_w <= 0.0 { 0.0 } else { rotated_x / rotated_w };
        let v = if rotated_h <= 0.0 {
            0.0
        } else {
            1.0 - (rotated_y / rotated_h)
        };

        egui::pos2(
            page_rect.left() + page_rect.width() * u,
            page_rect.top() + page_rect.height() * v,
        )
    }
}

fn normalized_selection_bounds(bounds: PdfTextBounds) -> PdfTextBounds {
    let min_width = 2.0;
    let min_height = 2.0;

    let center_x = (bounds.left + bounds.right) * 0.5;
    let center_y = (bounds.top + bounds.bottom) * 0.5;
    let half_width = (bounds.width() * 0.5).max(min_width * 0.5);
    let half_height = (bounds.height() * 0.5).max(min_height * 0.5);

    PdfTextBounds::from_points(
        center_x - half_width,
        center_x + half_width,
        center_y + half_height,
        center_y - half_height,
    )
}

fn normalize_selected_text(text: &str) -> String {
    text.replace("\r\n", "\n").trim().to_string()
}