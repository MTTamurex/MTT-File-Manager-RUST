use super::{chip_label, horizontal_drop_gap, paint_group_chip, CHIP_WIDTH};
use crate::domain::quick_access_item::destination_for_quick_access_gap;

#[test]
fn fixed_width_labels_fit_without_overlapping_the_pin() {
    let ctx = eframe::egui::Context::default();
    let _ = ctx.run_ui(Default::default(), |ui| {
        assert_eq!(CHIP_WIDTH, 124.0);
        for label in ["Documents", "Recycle Bin", "A", ""] {
            assert_eq!(chip_label(ui, label), label);
        }
        for label in [
            "A very long folder display name that needs truncation",
            "A very long translated recycle bin display name",
            "日本語のとても長いフォルダー表示名のテスト",
        ] {
            let visible_label = chip_label(ui, label);
            assert_ne!(visible_label, label);
            assert!(visible_label.ends_with("..."));
            let galley = ui.painter().layout_no_wrap(
                visible_label,
                eframe::egui::FontId::proportional(12.0),
                eframe::egui::Color32::WHITE,
            );
            assert!(galley.rect.width() <= CHIP_WIDTH - 56.0);
        }
    });
}

#[test]
fn reorder_destination_maps_horizontal_gaps_to_final_indices() {
    assert_eq!(destination_for_quick_access_gap(0, 0, 4), 0);
    assert_eq!(destination_for_quick_access_gap(0, 1, 4), 0);
    assert_eq!(destination_for_quick_access_gap(0, 2, 4), 1);
    assert_eq!(destination_for_quick_access_gap(2, 1, 4), 1);
    assert_eq!(destination_for_quick_access_gap(2, 3, 4), 2);
    assert_eq!(destination_for_quick_access_gap(2, 4, 4), 3);
}

#[test]
fn horizontal_drop_gap_uses_chip_centers_and_ignores_outside_pointer() {
    use eframe::egui::{pos2, vec2, Rect};

    let viewport = Rect::from_min_size(pos2(0.0, 0.0), vec2(500.0, 40.0));
    let rects = [
        Rect::from_min_size(pos2(20.0, 5.0), vec2(124.0, 26.0)),
        Rect::from_min_size(pos2(148.0, 5.0), vec2(124.0, 26.0)),
    ];

    assert_eq!(
        horizontal_drop_gap(Some(pos2(80.0, 15.0)), &rects, viewport),
        Some(0)
    );
    assert_eq!(
        horizontal_drop_gap(Some(pos2(100.0, 15.0)), &rects, viewport),
        Some(1)
    );
    assert_eq!(
        horizontal_drop_gap(Some(pos2(400.0, 15.0)), &rects, viewport),
        Some(2)
    );
    assert_eq!(
        horizontal_drop_gap(Some(pos2(600.0, 15.0)), &rects, viewport),
        None
    );
}

#[test]
fn saved_group_chip_paints_its_group_icon() {
    use eframe::egui::{pos2, vec2, RawInput, Sense};

    let ctx = eframe::egui::Context::default();
    let output = ctx.run_ui(
        RawInput {
            screen_rect: Some(eframe::egui::Rect::from_min_size(
                pos2(0.0, 0.0),
                vec2(400.0, 100.0),
            )),
            ..RawInput::default()
        },
        |ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(CHIP_WIDTH, 26.0), Sense::click());
            paint_group_chip(ui, rect, "Saved group", false);
        },
    );

    assert!(output.shapes.iter().any(|shape| match &shape.shape {
        eframe::egui::Shape::Text(text) => text.galley.job.text == "▤",
        _ => false,
    }));
}

/// Chips and the "+" slot must stay strictly inside the panel's top and bottom margins.
#[test]
fn quick_access_bar_row_stays_inside_panel_margins() {
    use eframe::egui;
    use eframe::egui::scroll_area::ScrollBarVisibility;
    use eframe::egui::{pos2, vec2, Color32, Margin, RawInput, Sense, Ui};
    use std::cell::Cell;

    let ctx = eframe::egui::Context::default();
    let bar_range = Cell::new((f32::NAN, f32::NAN));
    let chip_range = Cell::new((f32::NAN, f32::NAN));
    let add_range = Cell::new((f32::NAN, f32::NAN));

    let _ = ctx.run_ui(
        RawInput {
            screen_rect: Some(eframe::egui::Rect::from_min_size(
                pos2(0.0, 0.0),
                vec2(800.0, 600.0),
            )),
            ..Default::default()
        },
        |ui: &mut Ui| {
            egui::Panel::top("quick_access_bar")
                .show_separator_line(false)
                .exact_size(34.0)
                .frame(egui::Frame {
                    fill: Color32::from_rgb(243, 243, 243),
                    inner_margin: Margin {
                        left: 8,
                        right: 8,
                        top: 3,
                        bottom: 3,
                    },
                    ..Default::default()
                })
                .show(ui, |panel_ui: &mut Ui| {
                    let bar_rect = panel_ui.max_rect();
                    bar_range.set((bar_rect.top(), bar_rect.bottom()));

                    let scroll_output = egui::ScrollArea::horizontal()
                        .id_salt("quick_access_bar_scroll")
                        .auto_shrink([false, false])
                        .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
                        .show(panel_ui, |scroll_ui: &mut Ui| {
                            scroll_ui.horizontal(|row_ui: &mut Ui| {
                                row_ui.spacing_mut().item_spacing.x = 4.0;
                                let (add_rect, _) =
                                    row_ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
                                add_range.set((add_rect.top(), add_rect.bottom()));
                                let (chip_rect, _) = row_ui
                                    .allocate_exact_size(vec2(CHIP_WIDTH, 26.0), Sense::hover());
                                assert_eq!(chip_rect.width(), CHIP_WIDTH);
                                chip_range.set((chip_rect.top(), chip_rect.bottom()));
                            });
                        });
                    assert!(!scroll_output.inner_rect.is_negative());
                });
        },
    );

    let (bar_top, bar_bottom) = bar_range.get();
    let (chip_top, chip_bottom) = chip_range.get();
    let (add_top, add_bottom) = add_range.get();
    assert!(chip_top >= bar_top, "chip overlaps top margin");
    assert!(chip_bottom <= bar_bottom, "chip overflows bottom margin");
    assert!(add_top >= bar_top, "+ slot overlaps top margin");
    assert!(add_bottom <= bar_bottom, "+ slot overflows bottom margin");
}
