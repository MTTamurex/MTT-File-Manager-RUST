use super::strip::{self, StripLayout};
use eframe::egui;

fn render(
    ctx: &egui::Context,
    width: f32,
    active: usize,
    events: Vec<egui::Event>,
) -> (egui::Rect, egui::Rect, bool) {
    let mut active_rect = egui::Rect::NOTHING;
    let mut clip = egui::Rect::NOTHING;
    let mut reveal_active = false;
    let _ = ctx.run_ui(
        egui::RawInput {
            events,
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, 600.0),
            )),
            ..Default::default()
        },
        |ui| {
            let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 36.0));
            let layout = StripLayout::new(rect, 30);
            strip::show(ui, &layout, active, active, 30, |ui, reveal| {
                reveal_active = reveal;
                clip = ui.clip_rect();
                for index in 0..30 {
                    let (tab, _) = ui.allocate_exact_size(
                        egui::vec2(layout.tab_width, 36.0),
                        egui::Sense::click_and_drag(),
                    );
                    if index == active {
                        active_rect = tab;
                        if reveal {
                            ui.scroll_to_rect(tab, None);
                        }
                    }
                }
            });
            assert!(clip.right() <= layout.new_tab_and_drag.left());
            ui.scope_builder(egui::UiBuilder::new().max_rect(layout.controls), |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    for _ in 0..3 {
                        let (button, _) = ui.allocate_exact_size(
                            egui::vec2(strip::WINDOW_BUTTON_WIDTH, 36.0),
                            egui::Sense::click(),
                        );
                        assert!(button.right() <= rect.right() + 1.0);
                        assert!(button.left() >= layout.controls.left());
                    }
                });
            });
        },
    );
    (active_rect, clip, reveal_active)
}

#[test]
fn active_tab_is_revealed_after_selecting_and_resizing_at_multiple_scales() {
    for scale in [1.0, 1.25, 1.5, 2.0] {
        let ctx = egui::Context::default();
        ctx.set_pixels_per_point(scale);
        // Disable animation so scroll positions settle deterministically.
        ctx.all_styles_mut(|style| {
            style.animation_time = 0.0;
            style.scroll_animation = egui::style::ScrollAnimation::none();
        });
        for (width, active) in [(800.0, 0), (800.0, 29), (320.0, 29), (1920.0, 15)] {
            assert!(render(&ctx, width, active, vec![]).2);
            let mut result = render(&ctx, width, active, vec![]);
            for _ in 0..3 {
                result = render(&ctx, width, active, vec![]);
            }
            let (tab, clip, reveal) = result;
            assert!(!reveal, "unchanged state must not force scroll every frame");
            assert!(tab.left() >= clip.left() - 1.0, "{tab:?} outside {clip:?}");
            assert!(
                tab.right() <= clip.right() + 1.0,
                "{tab:?} outside {clip:?}"
            );
        }
    }
}

#[test]
fn mouse_wheel_scroll_is_not_reset_to_the_active_tab() {
    let ctx = egui::Context::default();
    for _ in 0..3 {
        render(&ctx, 800.0, 0, vec![]);
    }
    let before = render(&ctx, 800.0, 0, vec![]).0;
    render(
        &ctx,
        800.0,
        0,
        vec![
            egui::Event::PointerMoved(egui::pos2(100.0, 18.0)),
            egui::Event::MouseWheel {
                phase: egui::TouchPhase::Move,
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(-200.0, 0.0),
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    let mut after = before;
    for _ in 0..20 {
        let (tab, _, reveal) = render(&ctx, 800.0, 0, vec![]);
        assert!(!reveal);
        after = tab;
    }
    assert!(after.left() < before.left() - 50.0);
}
