use super::*;
use crate::ui::app::panels::sidebar_layout;
use egui::{vec2, Context, RawInput, Sense};

mod sections;

fn frame(
    ctx: &Context,
    unified: bool,
    section_heights: [f32; 3],
) -> egui::scroll_area::ScrollAreaOutput<()> {
    let mut result = None;
    let _ = ctx.run_ui(RawInput::default(), |ui| {
        ui.set_max_size(vec2(240.0, 220.0));
        let render = |ui: &mut egui::Ui| {
            for height in section_heights {
                ui.allocate_exact_size(vec2(220.0, height), Sense::hover());
            }
        };
        result = Some(if unified {
            scroll_area(ui, |ui, _| render(ui))
        } else {
            sidebar_layout::render_drives_block(ui, 220.0, false, 180.0, render).inner
        });
    });
    result.unwrap()
}

fn set_offset(ctx: &Context, output: &egui::scroll_area::ScrollAreaOutput<()>, offset: f32) {
    let mut state = output.state;
    state.offset.y = offset;
    state.store(ctx, output.id);
}

#[test]
fn every_section_contributes_its_full_height_to_the_unified_content() {
    let ctx = Context::default();
    let before = frame(&ctx, true, [300.0, 600.0, 400.0]);
    assert!(before.content_size.y >= 1300.0);
    assert!(before.inner_rect.height() <= 220.0);
    let after = frame(&ctx, true, [450.0, 900.0, 650.0]);
    assert!((after.content_size.y - before.content_size.y - 700.0).abs() < 1.0);
}

#[test]
fn collapsing_sections_clamps_the_scroll_offset_to_the_new_bounds() {
    let ctx = Context::default();
    let expanded = frame(&ctx, true, [300.0, 600.0, 400.0]);
    set_offset(&ctx, &expanded, 900.0);
    let scrolled = frame(&ctx, true, [300.0, 600.0, 400.0]);
    assert!(scrolled.state.offset.y > 0.0);

    // Allow the viewport to settle after its scrollbar visibility changes.
    frame(&ctx, true, [24.0, 30.0, 24.0]);
    let collapsed = frame(&ctx, true, [24.0, 30.0, 24.0]);
    assert_eq!(collapsed.state.offset.y, 0.0);
    assert!(collapsed.content_size.y < collapsed.inner_rect.height());
}

#[test]
fn switching_layouts_keeps_their_scroll_states_separate() {
    let ctx = Context::default();
    let independent = frame(&ctx, false, [300.0, 600.0, 400.0]);
    assert_eq!(independent.state.offset.y, 180.0);
    let unified = frame(&ctx, true, [300.0, 600.0, 400.0]);
    assert_ne!(unified.id, independent.id);
    assert_eq!(unified.state.offset.y, 0.0);
    set_offset(&ctx, &unified, 400.0);

    let independent_again = frame(&ctx, false, [300.0, 600.0, 400.0]);
    assert_eq!(independent_again.state.offset.y, 180.0);
    let unified_again = frame(&ctx, true, [300.0, 600.0, 400.0]);
    assert_eq!(unified_again.state.offset.y, 400.0);
}

#[test]
fn deep_content_can_scroll_horizontally_without_a_second_vertical_viewport() {
    let ctx = Context::default();
    let mut result = None;
    let _ = ctx.run_ui(RawInput::default(), |ui| {
        ui.set_max_size(vec2(240.0, 220.0));
        result = Some(scroll_area(ui, |ui, viewport_width| {
            assert!(viewport_width <= 240.0);
            ui.allocate_exact_size(vec2(900.0, 700.0), Sense::hover());
        }));
    });
    let output = result.unwrap();
    assert!(output.content_size.x > output.inner_rect.width());
    assert!(output.content_size.y > output.inner_rect.height());
}

#[test]
fn wide_content_does_not_expand_the_reported_viewport_on_later_frames() {
    let ctx = Context::default();
    for _ in 0..3 {
        let _ = ctx.run_ui(RawInput::default(), |ui| {
            ui.set_max_size(vec2(240.0, 220.0));
            scroll_area(ui, |ui, viewport_width| {
                assert!(
                    viewport_width <= 240.0,
                    "viewport width was {viewport_width}"
                );
                ui.allocate_exact_size(vec2(900.0, 700.0), Sense::hover());
            });
        });
    }
}
