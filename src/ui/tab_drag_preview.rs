//! Shared paint-only movement for reorderable strips and rows.
//! Hit testing and committed order stay unchanged.
use eframe::egui::{self, layers::ShapeIdx};

mod compositing;
mod floating;

pub(crate) use compositing::paint_root_background;

pub(crate) fn finish_frame(ctx: &egui::Context) {
    floating::finish_frame(ctx);
}

#[derive(Default)]
pub(crate) struct TabDragPreview {
    tabs: Vec<(egui::Rect, ShapeIdx, ShapeIdx)>,
}

#[derive(Clone, Copy)]
enum Axis {
    Horizontal,
    Vertical,
}

impl TabDragPreview {
    pub fn begin(ui: &egui::Ui) -> ShapeIdx {
        ui.ctx()
            .graphics_mut(|graphics| graphics.entry(ui.layer_id()).next_idx())
    }

    pub fn record(&mut self, ui: &egui::Ui, rect: egui::Rect, start: ShapeIdx) {
        self.tabs.push((rect, start, Self::begin(ui)));
    }

    pub fn paint(&self, ui: &egui::Ui, id: egui::Id, source: Option<usize>, strip: egui::Rect) {
        let pointer = ui.input(|input| input.pointer.interact_pos());
        let row = self
            .tabs
            .first()
            .zip(self.tabs.last())
            .map(|(first, last)| first.0.union(last.0).intersect(strip));
        let source = source.filter(|_| {
            !ui.input(|input| input.pointer.primary_released())
                || pointer.zip(row).is_some_and(|(pos, row)| row.contains(pos))
        });
        let gap = pointer
            .filter(|pos| row.is_some_and(|row| row.contains(*pos)))
            .map(|pos| {
                self.tabs
                    .iter()
                    .position(|(rect, _, _)| pos.x < rect.center().x)
                    .unwrap_or(self.tabs.len())
            });
        if let Some(index) = source.filter(|&index| index < self.tabs.len()) {
            let native = crate::ui::tab_bar::detach::pointer(ui.ctx());
            let outside_row = native.is_some_and(|pointer| pointer.outside)
                || !pointer.zip(row).is_some_and(|(pos, row)| row.contains(pos));
            if outside_row && floating::show(ui, self.tabs[index], native) {
                self.paint_at_gap(ui, id, None, strip, None);
                let (_, start, end) = self.tabs[index];
                ui.ctx().graphics_mut(|graphics| {
                    let list = graphics.entry(ui.layer_id());
                    for index in start.0..end.0 {
                        list.reset_shape(ShapeIdx(index));
                    }
                });
                ui.ctx().set_cursor_icon(egui::CursorIcon::Default);
                ui.ctx().request_repaint();
                return;
            }
        }
        self.paint_at_gap(ui, id, source, strip, gap);
    }

    /// Paint using the caller's drop gap, including blank space after the last header.
    pub fn paint_at_gap(
        &self,
        ui: &egui::Ui,
        id: egui::Id,
        source: Option<usize>,
        strip: egui::Rect,
        gap: Option<usize>,
    ) {
        self.paint_at_gap_on_axis(ui, id, source, strip, gap, None, Axis::Horizontal);
    }

    pub fn paint_at_vertical_gap(
        &self,
        ui: &egui::Ui,
        id: egui::Id,
        source: Option<usize>,
        strip: egui::Rect,
        gap: Option<usize>,
        source_grab_offset_y: Option<f32>,
    ) {
        self.paint_at_gap_on_axis(
            ui,
            id,
            source,
            strip,
            gap,
            source_grab_offset_y,
            Axis::Vertical,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_at_gap_on_axis(
        &self,
        ui: &egui::Ui,
        id: egui::Id,
        source: Option<usize>,
        strip: egui::Rect,
        gap: Option<usize>,
        source_grab_offset_y: Option<f32>,
        axis: Axis,
    ) {
        let pointer = ui.input(|input| input.pointer.interact_pos());
        let source = source.filter(|&index| index < self.tabs.len()).filter(|_| {
            !ui.input(|input| input.pointer.primary_released())
                || pointer.is_some_and(|pos| strip.contains(pos))
        });
        let gap = gap.filter(|&gap| gap <= self.tabs.len());
        let mut lifted = Vec::new();
        for (index, &(rect, start, end)) in self.tabs.iter().enumerate() {
            let target = source.zip(gap).map_or(0.0, |(source, gap)| {
                neighbor_offset(index, source, gap, &self.tabs, axis)
            });
            let animation = id.with(index);
            let offset = ui.ctx().animate_value_with_time(
                animation,
                target,
                if source.is_some() { 0.12 } else { 0.0 },
            );
            // egui returns the previous value on the frame the target changes,
            // even with a zero duration. Never apply it to the committed order.
            let offset = if source.is_some() { offset } else { 0.0 };
            let is_source = source == Some(index) && pointer.is_some();
            let delta = if is_source {
                let origin = ui
                    .input(|input| input.pointer.press_origin())
                    .unwrap_or(rect.center());
                match axis {
                    Axis::Horizontal => {
                        let x = (rect.left() + pointer.unwrap().x - origin.x).clamp(
                            strip.left(),
                            (strip.right() - rect.width()).max(strip.left()),
                        );
                        egui::vec2(x - rect.left(), 0.0)
                    }
                    Axis::Vertical => {
                        let grab_offset_y =
                            source_grab_offset_y.unwrap_or_else(|| origin.y - rect.top());
                        let y = (pointer.unwrap().y - grab_offset_y).clamp(
                            strip.top(),
                            (strip.bottom() - rect.height()).max(strip.top()),
                        );
                        egui::vec2(0.0, y - rect.top())
                    }
                }
            } else {
                match axis {
                    Axis::Horizontal => egui::vec2(offset, 0.0),
                    Axis::Vertical => egui::vec2(0.0, offset),
                }
            };
            if !is_source && delta == egui::Vec2::ZERO {
                continue;
            }
            ui.ctx().graphics_mut(|graphics| {
                let list = graphics.entry(ui.layer_id());
                for shape in start.0..end.0 {
                    list.mutate_shape(ShapeIdx(shape), |entry| {
                        entry.shape.translate(delta);
                        if is_source {
                            lifted.push(entry.clone());
                            entry.shape = egui::Shape::Noop;
                        }
                    });
                }
            });
            if is_source {
                let painter = ui.painter().with_clip_rect(strip);
                let raised = rect.translate(delta);
                painter.add(
                    egui::epaint::Shadow {
                        offset: [0, 2],
                        blur: 8,
                        spread: 1,
                        color: egui::Color32::from_black_alpha(65),
                    }
                    .as_shape(raised, 8),
                );
            }
        }
        // Append last so the dragged tab covers its sliding neighbors.
        if !lifted.is_empty() {
            ui.ctx().graphics_mut(|graphics| {
                let list = graphics.entry(ui.layer_id());
                for entry in lifted {
                    list.add(entry.clip_rect.intersect(strip), entry.shape);
                }
            });
        }
        if source.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Default);
            ui.ctx().request_repaint();
        }
    }
}

fn neighbor_offset(
    index: usize,
    source: usize,
    gap: usize,
    tabs: &[(egui::Rect, ShapeIdx, ShapeIdx)],
    axis: Axis,
) -> f32 {
    let extent = |rect: egui::Rect| match axis {
        Axis::Horizontal => rect.width(),
        Axis::Vertical => rect.height(),
    };
    let gap_after = |left: egui::Rect, right: egui::Rect| match axis {
        Axis::Horizontal => right.left() - left.right(),
        Axis::Vertical => right.top() - left.bottom(),
    };

    if index < source && index >= gap {
        extent(tabs[source].0) + gap_after(tabs[index].0, tabs[index + 1].0)
    } else if index > source && index < gap {
        -(extent(tabs[source].0) + gap_after(tabs[index - 1].0, tabs[index].0))
    } else {
        0.0
    }
}

#[cfg(test)]
#[path = "tab_drag_preview/tests.rs"]
mod tests;
