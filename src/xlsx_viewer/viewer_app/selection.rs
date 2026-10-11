use clipboard_win::{formats, Clipboard, Setter};
use eframe::egui;
use rust_i18n::t;

use super::super::cell_selection::CellSelection;
use super::super::renderer::XlsxCellBounds;
use super::{RenderKey, XlsxViewerApp};

pub(super) struct DragSelection {
    sheet_index: usize,
    anchor: (f32, f32),
    current: (f32, f32),
    anchor_screen: egui::Pos2,
    current_screen: egui::Pos2,
}

pub(super) struct SheetSelection {
    sheet_index: usize,
    range: betteroffice_xlsx::CellRange,
    bounds: XlsxCellBounds,
    text: String,
    frozen_width: f32,
    frozen_height: f32,
}

impl From<CellSelection> for SheetSelection {
    fn from(selection: CellSelection) -> Self {
        Self {
            sheet_index: selection.sheet_index,
            range: selection.range,
            bounds: selection.bounds,
            text: selection.text,
            frozen_width: selection.frozen_width,
            frozen_height: selection.frozen_height,
        }
    }
}

impl XlsxViewerApp {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn handle_sheet_selection(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        key: RenderKey,
        clip_origin: egui::Pos2,
        pixels_per_point: f32,
        frozen_width: f32,
        frozen_height: f32,
    ) {
        if response.drag_started() {
            ui.ctx().memory_mut(|memory| memory.stop_text_input());
            self.search_input_has_focus = false;
            if let Some(pos) = response.interact_pointer_pos() {
                if let Some(point) = point_in_sheet(
                    pos,
                    ui.clip_rect(),
                    key,
                    self.zoom,
                    pixels_per_point,
                    frozen_width,
                    frozen_height,
                ) {
                    self.selection_generation = self.selection_generation.wrapping_add(1);
                    self.selection = None;
                    self.drag_selection = Some(DragSelection {
                        sheet_index: key.sheet_index,
                        anchor: point,
                        current: point,
                        anchor_screen: pos,
                        current_screen: pos,
                    });
                }
            }
        }

        if let Some(pos) = response.interact_pointer_pos() {
            if let Some(drag) = self.drag_selection.as_mut() {
                if drag.sheet_index == key.sheet_index {
                    if let Some(point) = point_in_sheet(
                        pos,
                        ui.clip_rect(),
                        key,
                        self.zoom,
                        pixels_per_point,
                        frozen_width,
                        frozen_height,
                    ) {
                        drag.current = point;
                        drag.current_screen = pos;
                    }
                }
            }
        }

        if response.drag_stopped() {
            if let Some(drag) = self.drag_selection.take() {
                if drag.sheet_index == key.sheet_index {
                    self.request_range_selection(key.sheet_index, drag.anchor, drag.current);
                }
            }
        } else if response.clicked() && !response.dragged() {
            ui.ctx().memory_mut(|memory| memory.stop_text_input());
            self.search_input_has_focus = false;
            self.selection_generation = self.selection_generation.wrapping_add(1);
            self.drag_selection = None;
            self.selection = None;
        }

        self.paint_sheet_selection(
            ui,
            key,
            clip_origin,
            pixels_per_point,
            frozen_width,
            frozen_height,
        );
    }

    fn request_range_selection(&mut self, sheet_index: usize, start: (f32, f32), end: (f32, f32)) {
        self.selection_generation = self.selection_generation.wrapping_add(1);
        let generation = self.selection_generation;
        let requested = self.worker.as_ref().is_some_and(|worker| {
            worker.request_range_selection(generation, sheet_index, start, end)
        });
        if !requested {
            log::warn!("[XLSX-VIEWER] could not queue selected range for copying");
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
                log::warn!("[XLSX-VIEWER] copy selection failed: {error}");
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
            .ok_or_else(|| t!("xlsxviewer.selection_missing").to_string())?;
        if let Ok(_clipboard) = Clipboard::new_attempts(3) {
            if formats::Unicode.write_clipboard(text).is_ok() {
                return Ok(());
            }
        }
        Err(t!("operations.error_clipboard").to_string())
    }

    pub(super) fn selection_summary(&self) -> String {
        match self.selection.as_ref() {
            Some(selection) if !selection.text.is_empty() => {
                let rows = u64::from(selection.range.end.row - selection.range.start.row) + 1;
                let cols = u64::from(selection.range.end.col - selection.range.start.col) + 1;
                t!(
                    "xlsxviewer.selection_ready",
                    count = rows.saturating_mul(cols)
                )
                .to_string()
            }
            _ => t!("xlsxviewer.selection_hint").to_string(),
        }
    }

    fn paint_sheet_selection(
        &self,
        ui: &egui::Ui,
        key: RenderKey,
        clip_origin: egui::Pos2,
        pixels_per_point: f32,
        frozen_width: f32,
        frozen_height: f32,
    ) {
        let painter = ui.painter();
        if let Some(drag) = self
            .drag_selection
            .as_ref()
            .filter(|drag| drag.sheet_index == key.sheet_index)
        {
            let rect = egui::Rect::from_two_pos(drag.anchor_screen, drag.current_screen)
                .intersect(ui.clip_rect());
            if rect.is_positive() {
                paint_selection(painter, rect);
            }
        }

        let Some(selection) = self
            .selection
            .as_ref()
            .filter(|selection| selection.sheet_index == key.sheet_index)
        else {
            return;
        };
        let scale = self.zoom / pixels_per_point.max(0.5);
        let x_segments = visible_axis_segments(
            selection.bounds.left,
            selection.bounds.right,
            selection.frozen_width.max(frozen_width),
            key.viewport.x,
            ui.clip_rect().width() / scale,
        );
        let y_segments = visible_axis_segments(
            selection.bounds.top,
            selection.bounds.bottom,
            selection.frozen_height.max(frozen_height),
            key.viewport.y,
            ui.clip_rect().height() / scale,
        );
        for (left, right) in &x_segments {
            for (top, bottom) in &y_segments {
                let rect = egui::Rect::from_min_max(
                    clip_origin + egui::vec2(left * scale, top * scale),
                    clip_origin + egui::vec2(right * scale, bottom * scale),
                )
                .intersect(ui.clip_rect());
                if rect.is_positive() {
                    paint_selection(painter, rect);
                }
            }
        }
    }
}

fn point_in_sheet(
    pos: egui::Pos2,
    clip_rect: egui::Rect,
    key: RenderKey,
    zoom: f32,
    pixels_per_point: f32,
    frozen_width: f32,
    frozen_height: f32,
) -> Option<(f32, f32)> {
    if !clip_rect.contains(pos) {
        return None;
    }
    let scale = zoom / pixels_per_point.max(0.5);
    if !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    let local = (pos - clip_rect.min) / scale;
    let x = if local.x < frozen_width {
        local.x
    } else {
        key.viewport.x + local.x
    };
    let y = if local.y < frozen_height {
        local.y
    } else {
        key.viewport.y + local.y
    };
    Some((x, y))
}

fn visible_axis_segments(
    start: f32,
    end: f32,
    frozen: f32,
    scroll: f32,
    visible_extent: f32,
) -> Vec<(f32, f32)> {
    let mut segments = Vec::with_capacity(2);
    if start < frozen {
        let left = start.max(0.0);
        let right = end.min(frozen).min(visible_extent);
        if right > left {
            segments.push((left, right));
        }
    }
    if end > frozen {
        let left = (start.max(frozen) - scroll).max(frozen);
        let right = (end - scroll).min(visible_extent);
        if right > left {
            segments.push((left, right));
        }
    }
    segments
}

fn paint_selection(painter: &egui::Painter, rect: egui::Rect) {
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

#[cfg(test)]
mod tests {
    use super::{point_in_sheet, visible_axis_segments};
    use crate::xlsx_viewer::viewer_app::RenderKey;
    use betteroffice_xlsx::Viewport;
    use eframe::egui;

    #[test]
    fn pointer_coordinates_account_for_zoom_scroll_and_frozen_panes() {
        let clip = egui::Rect::from_min_size(egui::pos2(100.0, 200.0), egui::vec2(400.0, 300.0));
        let key = RenderKey {
            sheet_index: 0,
            viewport: Viewport {
                x: 100.0,
                y: 80.0,
                width: 200.0,
                height: 150.0,
            },
            zoom: 2.0,
        };

        assert_eq!(
            point_in_sheet(egui::pos2(120.0, 220.0), clip, key, 2.0, 1.0, 60.0, 40.0,),
            Some((10.0, 10.0)),
        );
        assert_eq!(
            point_in_sheet(egui::pos2(230.0, 290.0), clip, key, 2.0, 1.0, 60.0, 40.0,),
            Some((165.0, 125.0)),
        );
    }

    #[test]
    fn selected_range_is_split_across_frozen_and_scrolling_panes() {
        assert_eq!(
            visible_axis_segments(20.0, 180.0, 60.0, 100.0, 300.0),
            [(20.0, 60.0), (60.0, 80.0)]
        );
    }

    #[test]
    fn offscreen_body_ranges_do_not_paint_over_frozen_panes() {
        assert!(visible_axis_segments(80.0, 120.0, 60.0, 100.0, 300.0).is_empty());
    }
}
