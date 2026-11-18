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
                let offset = neighbor_offset(index, source, gap, &tabs, Axis::Horizontal);
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
                let rect = egui::Rect::from_min_size(egui::pos2(x, 0.0), egui::vec2(width, 22.0));
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
                            tabs[index].0.left()
                                + neighbor_offset(index, source, gap, &tabs, Axis::Horizontal,),
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
fn vertical_drag_preview_stays_anchored_to_the_pointer_while_rows_scroll() {
    let ctx = egui::Context::default();
    let strip = egui::Rect::from_min_size(egui::pos2(10.0, 10.0), egui::vec2(180.0, 100.0));
    let press = egui::pos2(30.0, 20.0);
    let pointer = egui::pos2(30.0, 70.0);
    let grab_offset_y = press.y - strip.top();
    let mut shapes = Vec::new();
    for (events, scroll_y) in [
        (
            vec![
                egui::Event::PointerMoved(press),
                egui::Event::PointerButton {
                    pos: press,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            0.0,
        ),
        (vec![egui::Event::PointerMoved(pointer)], 35.0),
        (vec![egui::Event::PointerMoved(pointer)], 55.0),
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
                        strip.min + egui::vec2(0.0, index as f32 * 30.0 - scroll_y),
                        egui::vec2(180.0, 28.0),
                    );
                    let start = TabDragPreview::begin(ui);
                    ui.painter().rect_filled(
                        rect,
                        0.0,
                        if index == 0 {
                            egui::Color32::RED
                        } else {
                            egui::Color32::BLUE
                        },
                    );
                    preview.record(ui, rect, start);
                }
                preview.paint_at_vertical_gap(
                    ui,
                    egui::Id::new("vertical_preview_test"),
                    Some(0),
                    strip,
                    Some(2),
                    Some(grab_offset_y),
                );
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
        panic!("expected lifted row");
    };
    assert_eq!(rect.fill, egui::Color32::RED);
    assert_eq!(rect.rect.top(), pointer.y - grab_offset_y);
    assert_eq!(lifted.clip_rect, strip);
}

#[test]
fn vertical_neighbor_offsets_use_row_heights_and_vertical_spacing() {
    let rows: Vec<_> = (0..3)
        .map(|index| {
            (
                egui::Rect::from_min_size(
                    egui::pos2(0.0, index as f32 * 30.0),
                    egui::vec2(180.0, 28.0),
                ),
                ShapeIdx(0),
                ShapeIdx(0),
            )
        })
        .collect();

    assert_eq!(neighbor_offset(1, 0, 3, &rows, Axis::Vertical), -30.0);
    assert_eq!(neighbor_offset(2, 0, 3, &rows, Axis::Vertical), -30.0);
    assert_eq!(neighbor_offset(1, 2, 0, &rows, Axis::Vertical), 30.0);
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
                let entries: Vec<_> = graphics.get(ui.layer_id()).unwrap().all_entries().collect();
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
