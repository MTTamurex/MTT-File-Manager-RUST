use super::{columns, render_row, HeaderRow, HeaderSpec};
use crate::domain::file_entry::SortMode;
use crate::domain::list_columns::{ColumnId, ColumnLayout, ColumnScope};
use eframe::egui;

fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        pressed,
        button: egui::PointerButton::Primary,
        modifiers: egui::Modifiers::NONE,
    }
}

fn frame(
    ctx: &egui::Context,
    layout: &mut ColumnLayout,
    events: Vec<egui::Event>,
    pane: &str,
    clip: Option<egui::Rect>,
    focused: bool,
) -> (HeaderRow, egui::FullOutput) {
    let mut row = None;
    let output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 400.0),
            )),
            events,
            focused,
            ..Default::default()
        },
        |ui| {
            ui.push_id(pane, |ui| {
                if let Some(clip) = clip {
                    ui.set_clip_rect(clip);
                }
                let mut offset = 0.0;
                let headers = layout
                    .columns
                    .iter()
                    .filter(|column| column.visible)
                    .map(|column| {
                        let cell = columns::ColumnCell {
                            id: column.id,
                            width: column.width,
                            offset,
                        };
                        offset += cell.width;
                        HeaderSpec {
                            cell,
                            label: format!("{:?}", column.id),
                            mode: columns::sort_mode(column.id),
                        }
                    })
                    .collect();
                row = Some(render_row(
                    ui,
                    layout,
                    headers,
                    850.0,
                    SortMode::Name,
                    false,
                ));
                // A data-cell stand-in must never be captured by the header animation.
                ui.painter().text(
                    egui::pos2(10.0, 80.0),
                    egui::Align2::LEFT_TOP,
                    "cell",
                    egui::FontId::proportional(12.0),
                    egui::Color32::WHITE,
                );
            });
        },
    );
    (row.unwrap(), output)
}

fn text_position(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
    output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == label => Some(text.pos),
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing label: {label}"))
}

fn start_drag(ctx: &egui::Context, layout: &mut ColumnLayout, start: egui::Pos2) {
    frame(ctx, layout, vec![], "left", None, true);
    frame(
        ctx,
        layout,
        vec![egui::Event::PointerMoved(start)],
        "left",
        None,
        true,
    );
    frame(ctx, layout, vec![button(start, true)], "left", None, true);
}

#[test]
fn only_headers_move_with_an_arrow_cursor_and_commit_on_release() {
    let ctx = egui::Context::default();
    let mut layout = ColumnLayout::defaults(ColumnScope::Computer);
    layout.columns[1].width = 120.0;
    let original = layout.clone();
    let start = egui::pos2(70.0, 12.0);
    let end = egui::pos2(700.0, 12.0); // blank header area remains a valid end gap
    start_drag(&ctx, &mut layout, start);
    let (_, baseline) = frame(&ctx, &mut layout, vec![], "left", None, true);
    let (row, output) = frame(
        &ctx,
        &mut layout,
        vec![egui::Event::PointerMoved(end)],
        "left",
        None,
        true,
    );
    assert_eq!(layout, original);
    assert_eq!(row.sort, None);
    assert!(row.resized.is_empty());
    assert_eq!(
        output.platform_output.cursor_icon,
        egui::CursorIcon::Default
    );
    assert_eq!(
        text_position(&output, "Name").x - text_position(&baseline, "Name").x,
        (end.x - start.x).min(850.0 - original.columns[0].width)
    );
    assert_eq!(
        text_position(&output, "cell"),
        text_position(&baseline, "cell")
    );
    // Let the neighboring headers reach their animated targets.
    let mut settled = output;
    for _ in 0..12 {
        settled = frame(&ctx, &mut layout, vec![], "left", None, true).1;
    }
    assert_eq!(
        text_position(&settled, "TotalSpace").x,
        text_position(&baseline, "TotalSpace").x - 300.0
    );
    assert_eq!(
        text_position(&settled, "FreeSpace").x,
        text_position(&baseline, "FreeSpace").x - 300.0
    );
    assert_eq!(layout, original);
    let (row, _) = frame(
        &ctx,
        &mut layout,
        vec![button(end, false)],
        "left",
        None,
        true,
    );
    assert_eq!(row.sort, None);
    assert_eq!(layout.columns.last().unwrap().id, ColumnId::Name);
    let (_, committed) = frame(&ctx, &mut layout, vec![], "left", None, true);
    assert_eq!(
        text_position(&committed, "TotalSpace").x,
        text_position(&baseline, "Name").x
    );
}

#[test]
fn escape_focus_loss_and_outside_release_cancel_without_changing_data_order() {
    for cancellation in 0..3 {
        let ctx = egui::Context::default();
        let mut layout = ColumnLayout::defaults(ColumnScope::Computer);
        let original = layout.clone();
        let start = egui::pos2(70.0, 12.0);
        let end = egui::pos2(700.0, 12.0);
        start_drag(&ctx, &mut layout, start);
        frame(
            &ctx,
            &mut layout,
            vec![egui::Event::PointerMoved(end)],
            "left",
            None,
            true,
        );
        match cancellation {
            0 => {
                frame(
                    &ctx,
                    &mut layout,
                    vec![egui::Event::Key {
                        key: egui::Key::Escape,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                    "left",
                    None,
                    true,
                );
            }
            1 => {
                frame(&ctx, &mut layout, vec![], "left", None, false);
            }
            _ => {
                frame(
                    &ctx,
                    &mut layout,
                    vec![egui::Event::PointerMoved(egui::pos2(700.0, 100.0))],
                    "left",
                    None,
                    true,
                );
            }
        }
        let release = if cancellation == 2 {
            egui::pos2(700.0, 100.0)
        } else {
            end
        };
        frame(
            &ctx,
            &mut layout,
            vec![button(release, false)],
            "left",
            None,
            true,
        );
        assert_eq!(layout, original);
    }
}

#[test]
fn clipping_limits_the_preview_and_rejects_invisible_drop_targets() {
    let ctx = egui::Context::default();
    let mut layout = ColumnLayout::defaults(ColumnScope::Computer);
    let original = layout.clone();
    let start = egui::pos2(70.0, 12.0);
    let end = egui::pos2(700.0, 12.0);
    let clip = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 200.0));
    start_drag(&ctx, &mut layout, start);
    let (_, output) = frame(
        &ctx,
        &mut layout,
        vec![egui::Event::PointerMoved(end)],
        "left",
        Some(clip),
        true,
    );
    let lifted = output
        .shapes
        .iter()
        .find(|shape| {
            matches!(&shape.shape,
        egui::Shape::Text(text) if text.galley.job.text == "Name")
        })
        .unwrap();
    assert!(lifted.clip_rect.right() <= clip.right());
    frame(
        &ctx,
        &mut layout,
        vec![button(end, false)],
        "left",
        Some(clip),
        true,
    );
    assert_eq!(layout, original);
}

#[test]
fn resize_keeps_its_cursor_and_never_starts_reordering() {
    let ctx = egui::Context::default();
    let mut layout = ColumnLayout::defaults(ColumnScope::Computer);
    let original = layout.clone();
    let start = egui::pos2(299.0, 12.0);
    start_drag(&ctx, &mut layout, start);
    let (row, output) = frame(
        &ctx,
        &mut layout,
        vec![egui::Event::PointerMoved(start + egui::vec2(30.0, 0.0))],
        "left",
        None,
        true,
    );
    assert_eq!(row.sort, None);
    assert_eq!(row.resized, vec![(ColumnId::Name, 330.0)]);
    assert_eq!(
        output.platform_output.cursor_icon,
        egui::CursorIcon::ResizeHorizontal
    );
    assert_eq!(layout, original);
}

#[test]
fn dragging_in_one_pane_does_not_reorder_another_pane() {
    let ctx = egui::Context::default();
    let mut left = ColumnLayout::defaults(ColumnScope::Computer);
    let mut right = left.clone();
    let original = right.clone();
    let start = egui::pos2(70.0, 12.0);
    let end = egui::pos2(700.0, 12.0);
    start_drag(&ctx, &mut left, start);
    frame(
        &ctx,
        &mut left,
        vec![egui::Event::PointerMoved(end)],
        "left",
        None,
        true,
    );
    frame(
        &ctx,
        &mut right,
        vec![button(end, false)],
        "right",
        None,
        true,
    );
    assert_eq!(right, original);
}

#[test]
fn moving_a_narrow_header_left_preserves_hidden_columns_and_widths() {
    let ctx = egui::Context::default();
    let mut layout = ColumnLayout::defaults(ColumnScope::Computer);
    layout.set_visible(ColumnId::TotalSpace, false);
    let original = layout.clone();
    let start = egui::pos2(340.0, 12.0);
    let end = egui::pos2(70.0, 12.0);
    start_drag(&ctx, &mut layout, start);
    let (_, baseline) = frame(&ctx, &mut layout, vec![], "left", None, true);
    frame(
        &ctx,
        &mut layout,
        vec![egui::Event::PointerMoved(end)],
        "left",
        None,
        true,
    );
    let mut output = baseline.clone();
    for _ in 0..12 {
        output = frame(&ctx, &mut layout, vec![], "left", None, true).1;
    }
    assert_eq!(layout, original);
    assert_eq!(
        text_position(&output, "Name").x,
        text_position(&baseline, "Name").x + 160.0
    );
    assert_eq!(
        text_position(&output, "FreeSpace").x - text_position(&baseline, "FreeSpace").x,
        end.x - start.x
    );
    let (row, _) = frame(
        &ctx,
        &mut layout,
        vec![button(end, false)],
        "left",
        None,
        true,
    );
    assert_eq!(row.sort, None);
    assert_eq!(layout.columns[0], original.columns[2]);
    assert_eq!(layout.columns[1], original.columns[0]);
    assert_eq!(layout.columns[2], original.columns[1]);
    assert!(!layout.is_visible(ColumnId::TotalSpace));
}
