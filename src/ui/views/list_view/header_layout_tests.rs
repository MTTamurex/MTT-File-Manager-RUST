use super::{render_row, HeaderSpec};
use crate::domain::file_entry::SortMode;
use crate::domain::list_columns::{ColumnId, ColumnLayout, ColumnScope};
use crate::ui::views::list_view::columns::ColumnCell;
use eframe::egui;

fn button(pos: egui::Pos2, pressed: bool, button: egui::PointerButton) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        pressed,
        button,
        modifiers: egui::Modifiers::NONE,
    }
}

fn frame(
    ctx: &egui::Context,
    layout: &mut ColumnLayout,
    pane: &str,
    events: Vec<egui::Event>,
    menu: &mut bool,
) -> Option<SortMode> {
    let mut sort = None;
    let _ = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 400.0),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            ui.push_id(pane, |ui| {
                let mut offset = 0.0;
                let headers = layout
                    .columns
                    .iter()
                    .filter(|column| column.visible)
                    .map(|column| {
                        let cell = ColumnCell {
                            id: column.id,
                            width: 160.0,
                            offset,
                        };
                        offset += cell.width;
                        HeaderSpec {
                            cell,
                            label: format!("{:?}", column.id),
                            mode: super::columns::sort_mode(column.id),
                        }
                    })
                    .collect();
                let row = render_row(ui, layout, headers, 850.0, SortMode::Name, false);
                sort = row.sort;
                for response in row.responses {
                    response.context_menu(|ui| {
                        *menu = true;
                        ui.label("Columns");
                    });
                }
            });
        },
    );
    sort
}

#[test]
fn name_can_move_to_the_end_without_sorting() {
    let ctx = egui::Context::default();
    let mut layout = ColumnLayout::defaults(ColumnScope::Computer);
    let mut menu = false;
    let start = egui::pos2(70.0, 12.0);
    let end = egui::pos2(600.0, 12.0);
    frame(&ctx, &mut layout, "left", vec![], &mut menu);
    frame(
        &ctx,
        &mut layout,
        "left",
        vec![egui::Event::PointerMoved(start)],
        &mut menu,
    );
    frame(
        &ctx,
        &mut layout,
        "left",
        vec![button(start, true, egui::PointerButton::Primary)],
        &mut menu,
    );
    assert_eq!(
        frame(
            &ctx,
            &mut layout,
            "left",
            vec![egui::Event::PointerMoved(end)],
            &mut menu
        ),
        None
    );
    assert_eq!(layout.columns[0].id, ColumnId::Name);
    assert_eq!(
        frame(
            &ctx,
            &mut layout,
            "left",
            vec![button(end, false, egui::PointerButton::Primary)],
            &mut menu
        ),
        None
    );
    assert_eq!(layout.columns.last().unwrap().id, ColumnId::Name);
    assert!(layout.is_visible(ColumnId::Name));
}

#[test]
fn releasing_outside_the_header_cancels_reordering() {
    let ctx = egui::Context::default();
    let mut layout = ColumnLayout::defaults(ColumnScope::Computer);
    let original = layout.clone();
    let mut menu = false;
    let start = egui::pos2(70.0, 12.0);
    let end = egui::pos2(600.0, 100.0);
    frame(&ctx, &mut layout, "left", vec![], &mut menu);
    frame(
        &ctx,
        &mut layout,
        "left",
        vec![egui::Event::PointerMoved(start)],
        &mut menu,
    );
    frame(
        &ctx,
        &mut layout,
        "left",
        vec![button(start, true, egui::PointerButton::Primary)],
        &mut menu,
    );
    frame(
        &ctx,
        &mut layout,
        "left",
        vec![egui::Event::PointerMoved(end)],
        &mut menu,
    );
    frame(
        &ctx,
        &mut layout,
        "left",
        vec![button(end, false, egui::PointerButton::Primary)],
        &mut menu,
    );
    assert_eq!(layout, original);
}

#[test]
fn right_click_menu_opens_on_header_body_resize_edge_and_blank_bar() {
    for x in [70.0, 160.0, 700.0] {
        let ctx = egui::Context::default();
        let mut layout = ColumnLayout::defaults(ColumnScope::Computer);
        let original = layout.clone();
        let mut menu = false;
        let point = egui::pos2(x, 12.0);
        frame(&ctx, &mut layout, "left", vec![], &mut menu);
        frame(
            &ctx,
            &mut layout,
            "left",
            vec![egui::Event::PointerMoved(point)],
            &mut menu,
        );
        frame(
            &ctx,
            &mut layout,
            "left",
            vec![button(point, true, egui::PointerButton::Secondary)],
            &mut menu,
        );
        frame(
            &ctx,
            &mut layout,
            "left",
            vec![button(point, false, egui::PointerButton::Secondary)],
            &mut menu,
        );
        assert!(menu, "menu did not open at x={x}");
        assert_eq!(layout, original);
    }
}
