//! Shared paint-only movement for tabs and column headers.
//! Hit testing and committed order stay unchanged.
use eframe::egui::{self, layers::ShapeIdx};

#[derive(Default)]
pub(crate) struct TabDragPreview {
    tabs: Vec<(egui::Rect, ShapeIdx, ShapeIdx)>,
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
        let pointer = ui.input(|input| input.pointer.interact_pos());
        let source = source.filter(|&index| index < self.tabs.len()).filter(|_| {
            !ui.input(|input| input.pointer.primary_released())
                || pointer.is_some_and(|pos| strip.contains(pos))
        });
        let gap = gap.filter(|&gap| gap <= self.tabs.len());
        let mut lifted = Vec::new();
        for (index, &(rect, start, end)) in self.tabs.iter().enumerate() {
            let target = source.zip(gap).map_or(0.0, |(source, gap)| {
                neighbor_offset(index, source, gap, &self.tabs)
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
                let x = (rect.left() + pointer.unwrap().x - origin.x).clamp(
                    strip.left(),
                    (strip.right() - rect.width()).max(strip.left()),
                );
                egui::vec2(x - rect.left(), 0.0)
            } else {
                egui::vec2(offset, 0.0)
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
) -> f32 {
    if index < source && index >= gap {
        tabs[source].0.width() + (tabs[index + 1].0.left() - tabs[index].0.right())
    } else if index > source && index < gap {
        -(tabs[source].0.width() + (tabs[index].0.left() - tabs[index - 1].0.right()))
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neighbors_open_exactly_one_slot_in_both_directions() {
        let tabs: Vec<_> = (0..4)
            .map(|i| {
                (
                    egui::Rect::from_min_size(
                        egui::pos2(i as f32 * 102.0, 0.0),
                        egui::vec2(100.0, 28.0),
                    ),
                    ShapeIdx(0),
                    ShapeIdx(0),
                )
            })
            .collect();
        for source in 0..4 {
            for gap in 0..=4 {
                let destination = if gap > source { gap - 1 } else { gap };
                let mut occupied = std::collections::HashSet::new();
                for index in 0..4 {
                    if index == source {
                        continue;
                    }
                    let offset = neighbor_offset(index, source, gap, &tabs);
                    let slot = index as isize + (offset / 102.0) as isize;
                    assert_ne!(slot, destination as isize);
                    assert!((0..4).contains(&slot));
                    assert!(occupied.insert(slot));
                }
            }
        }
    }

    #[test]
    fn unequal_widths_open_a_gap_matching_the_source_width() {
        for spacing in [0.0, 2.0] {
            let mut x = 0.0;
            let tabs: Vec<_> = [300.0, 80.0, 160.0, 120.0]
                .into_iter()
                .map(|width| {
                    let rect =
                        egui::Rect::from_min_size(egui::pos2(x, 0.0), egui::vec2(width, 22.0));
                    x += width + spacing;
                    (rect, ShapeIdx(0), ShapeIdx(0))
                })
                .collect();
            for source in 0..tabs.len() {
                for gap in 0..=tabs.len() {
                    let mut order: Vec<_> = (0..tabs.len()).collect();
                    order.remove(source);
                    let destination = if gap > source { gap - 1 } else { gap };
                    order.insert(destination, source);
                    let mut x = 0.0;
                    for index in order {
                        if index != source {
                            assert_eq!(
                                tabs[index].0.left() + neighbor_offset(index, source, gap, &tabs),
                                x
                            );
                        }
                        x += tabs[index].0.width() + spacing;
                    }
                }
            }
        }
    }

    #[test]
    fn lifted_tab_preserves_grab_offset_clip_and_draws_above_neighbors() {
        let ctx = egui::Context::default();
        let strip = egui::Rect::from_min_size(egui::pos2(10.0, 10.0), egui::vec2(300.0, 28.0));
        let press = egui::pos2(30.0, 24.0);
        let pointer = egui::pos2(170.0, 24.0);
        let mut shapes = Vec::new();
        for events in [
            vec![
                egui::Event::PointerMoved(press),
                egui::Event::PointerButton {
                    pos: press,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            vec![egui::Event::PointerMoved(pointer)],
        ] {
            let _ = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    let mut preview = TabDragPreview::default();
                    for index in 0..3 {
                        let rect = egui::Rect::from_min_size(
                            strip.min + egui::vec2(index as f32 * 100.0, 0.0),
                            egui::vec2(100.0, 28.0),
                        );
                        let start = TabDragPreview::begin(ui);
                        ui.painter().with_clip_rect(strip).rect_filled(
                            rect,
                            8.0,
                            if index == 0 {
                                egui::Color32::RED
                            } else {
                                egui::Color32::BLUE
                            },
                        );
                        preview.record(ui, rect, start);
                    }
                    preview.paint(ui, egui::Id::new("preview_test"), Some(0), strip);
                    shapes = ctx.graphics(|graphics| {
                        graphics
                            .get(ui.layer_id())
                            .unwrap()
                            .all_entries()
                            .cloned()
                            .collect()
                    });
                },
            );
        }
        assert!(matches!(shapes[0].shape, egui::Shape::Noop));
        let lifted = shapes.last().unwrap();
        let egui::Shape::Rect(rect) = &lifted.shape else {
            panic!("expected lifted tab");
        };
        assert_eq!(rect.fill, egui::Color32::RED);
        assert_eq!(rect.rect.left(), pointer.x - (press.x - strip.left()));
        assert_eq!(rect.rect.top(), strip.top());
        assert_eq!(lifted.clip_rect, strip);
    }

    #[test]
    fn finishing_or_cancelling_never_applies_stale_animation_to_committed_order() {
        for source in [None, Some(99)] {
            let ctx = egui::Context::default();
            let id = egui::Id::new("reset_preview");
            let strip = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300.0, 28.0));
            let _ = ctx.run_ui(egui::RawInput::default(), |_| {
                for index in 0..3 {
                    ctx.animate_value_with_time(id.with(index), 100.0, 0.0);
                }
            });
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                let mut preview = TabDragPreview::default();
                for index in 0..3 {
                    let rect = egui::Rect::from_min_size(
                        egui::pos2(index as f32 * 100.0, 0.0),
                        egui::vec2(100.0, 28.0),
                    );
                    let start = TabDragPreview::begin(ui);
                    ui.painter().rect_filled(rect, 8.0, egui::Color32::BLUE);
                    preview.record(ui, rect, start);
                }
                preview.paint(ui, id, source, strip);
                ctx.graphics(|graphics| {
                    let entries: Vec<_> =
                        graphics.get(ui.layer_id()).unwrap().all_entries().collect();
                    assert_eq!(entries.len(), 3, "no lifted copy or shadow after reset");
                    for (index, entry) in entries.iter().enumerate() {
                        let egui::Shape::Rect(rect) = &entry.shape else {
                            panic!("expected tab");
                        };
                        assert_eq!(rect.rect.left(), index as f32 * 100.0);
                    }
                });
            });
        }
    }
}
