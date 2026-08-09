//! Headless reproduction of the image viewer scroll freeze bug.
//!
//! Mirrors the exact `ScrollArea` geometry used by
//! `DedicatedImageViewerApp::render_center` (src/image_viewer/app/rendering.rs):
//! a `ScrollArea::both()` whose `scroll_bar_rect` is a ZERO-HEIGHT rect pinned
//! to the bottom of the viewport, with content larger than the viewport.
//!
//! Simulates pointer press/drag on the scroll bars and reports whether egui
//! panics (the root cause of the "not responding" viewer window).

use eframe::egui;
use eframe::egui::scroll_area::ScrollBarVisibility;

const WINDOW: egui::Vec2 = egui::vec2(1200.0, 850.0);
/// Simulated zoomed image draw size (larger than the viewport on both axes).
const DRAW_SIZE: egui::Vec2 = egui::vec2(4800.0, 3600.0);

/// One frame with the exact scroll-area structure of render_center.
/// `degenerate` toggles the buggy zero-height scroll_bar_rect that existed
/// before the fix; the fixed code passes no scroll_bar_rect at all.
fn render_frame(
    root_ui: &mut egui::Ui,
    degenerate: bool,
    last_offset: &std::cell::Cell<Option<egui::Vec2>>,
) {
    egui::CentralPanel::default().show(root_ui, |ui| {
        let viewport_size = ui.available_size();

        let mut area = egui::ScrollArea::both()
            .id_salt("image_viewer_center_scroll")
            .auto_shrink([false, false])
            .scroll_bar_visibility(ScrollBarVisibility::VisibleWhenNeeded);

        if degenerate {
            // Old production code: zero-height rect pinned to the bottom edge.
            let available_rect = ui.available_rect_before_wrap();
            area = area.scroll_bar_rect(egui::Rect::from_min_max(
                egui::pos2(available_rect.left(), available_rect.bottom()),
                egui::pos2(available_rect.right(), available_rect.bottom()),
            ));
        }

        let output = area.show(ui, |ui| {
            let canvas_size = egui::vec2(
                DRAW_SIZE.x.max(viewport_size.x),
                DRAW_SIZE.y.max(viewport_size.y),
            );
            ui.set_min_size(canvas_size);
            let (canvas_rect, _) = ui.allocate_at_least(canvas_size, egui::Sense::hover());

            let image_rect = egui::Rect::from_center_size(canvas_rect.center(), DRAW_SIZE);

            // Same interaction shape as the image widget in render_center:
            // click = zoom, drag = pan.
            let response = ui.interact(
                image_rect,
                ui.id().with("image"),
                egui::Sense::click() | egui::Sense::drag(),
            );

            if response.dragged() {
                response.drag_delta()
            } else {
                egui::Vec2::ZERO
            }
        });

        // Same drag-to-pan state mutation as render_center: apply the image's
        // drag delta to the scroll area's persisted offset.
        let pan_delta = output.inner;
        if pan_delta != egui::Vec2::ZERO {
            let scroll_id = ui.make_persistent_id(egui::IdSalt::new("image_viewer_center_scroll"));
            if let Some(mut state) = egui::scroll_area::State::load(ui.ctx(), scroll_id) {
                state.offset -= pan_delta;
                state.store(ui.ctx(), scroll_id);
            }
        }

        last_offset.set(Some(output.state.offset));
    });
}

fn base_input() -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, WINDOW)),
        ..Default::default()
    }
}

fn run_ui(
    ctx: &egui::Context,
    input: egui::RawInput,
    degenerate: bool,
    last_offset: &std::cell::Cell<Option<egui::Vec2>>,
) {
    let _ = ctx.run_ui(input, |root_ui| {
        render_frame(root_ui, degenerate, last_offset);
    });
}

/// Press + drag + release at the given x/y (as fractions of the window).
/// Returns the final scroll offset when no panic occurred.
fn run_scenario(
    degenerate: bool,
    x_frac: f32,
    y_frac: f32,
    drag_delta: egui::Vec2,
) -> Result<Option<egui::Vec2>, String> {
    let ctx = egui::Context::default();
    let last_offset = std::cell::Cell::new(None);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // Initial frame to register widget rects.
        run_ui(&ctx, base_input(), degenerate, &last_offset);

        let screen = ctx.input(|i| i.viewport_rect());
        let press = egui::pos2(screen.width() * x_frac, screen.height() * y_frac);

        // Move pointer into position.
        let mut input = base_input();
        input.events.push(egui::Event::PointerMoved(press));
        run_ui(&ctx, input, degenerate, &last_offset);

        // Press.
        let mut input = base_input();
        input.events.push(egui::Event::PointerButton {
            pos: press,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        run_ui(&ctx, input, degenerate, &last_offset);

        // Drag.
        let drag = egui::pos2(press.x + drag_delta.x, press.y + drag_delta.y);
        let mut input = base_input();
        input.events.push(egui::Event::PointerMoved(drag));
        run_ui(&ctx, input, degenerate, &last_offset);

        // Release.
        let mut input = base_input();
        input.events.push(egui::Event::PointerButton {
            pos: drag,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        run_ui(&ctx, input, degenerate, &last_offset);

        // One extra frame so the stored pan offset is loaded back by the
        // ScrollArea (the mutation takes effect on the frame after it is made).
        run_ui(&ctx, base_input(), degenerate, &last_offset);
    }));

    match result {
        Ok(()) => Ok(last_offset.take()),
        Err(payload) => {
            let msg = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "<unknown panic payload>".into());
            Err(msg)
        }
    }
}

fn main() {
    // Silence the default panic hook noise; we report results ourselves.
    std::panic::set_hook(Box::new(|_| {}));

    let scenarios: [(&str, bool, f32, f32, egui::Vec2); 5] = [
        (
            "BUGGY rect | vertical bar press+drag  ",
            true,
            0.997,
            0.5,
            egui::vec2(0.0, 80.0),
        ),
        (
            "BUGGY rect | horizontal bar press+drag",
            true,
            0.5,
            0.995,
            egui::vec2(80.0, 0.0),
        ),
        (
            "FIXED code | vertical bar press+drag  ",
            false,
            0.997,
            0.5,
            egui::vec2(0.0, 80.0),
        ),
        (
            "FIXED code | horizontal bar press+drag",
            false,
            0.5,
            0.995,
            egui::vec2(80.0, 0.0),
        ),
        // Drag the image itself toward the top-left: content must scroll
        // down/right (positive offset on both axes).
        (
            "FIXED code | image drag-to-pan          ",
            false,
            0.5,
            0.5,
            egui::vec2(-80.0, -80.0),
        ),
    ];

    let mut failures = 0;
    let mut reproduced_bug = false;
    for (label, degenerate, x, y, drag) in scenarios {
        match run_scenario(degenerate, x, y, drag) {
            Ok(offset) => {
                let offset_str = offset
                    .map(|o| format!("({:.1}, {:.1})", o.x, o.y))
                    .unwrap_or_else(|| "?".into());
                println!("{label} => OK, final scroll offset = {offset_str}");
                if !degenerate {
                    // Functional check: the drag must move the content along
                    // the expected axis.
                    let moved = if label.contains("vertical") {
                        offset.is_some_and(|o| o.y > 1.0)
                    } else if label.contains("horizontal") {
                        offset.is_some_and(|o| o.x > 1.0)
                    } else {
                        offset.is_some_and(|o| o.x > 1.0 && o.y > 1.0)
                    };
                    if !moved {
                        println!("  !! FAIL: drag did not move the content as expected");
                        failures += 1;
                    }
                }
            }
            Err(msg) => {
                println!("{label} => PANIC: {msg}");
                if degenerate && label.contains("vertical") && msg.contains("min > max") {
                    reproduced_bug = true;
                } else if !degenerate || label.contains("vertical") {
                    failures += 1;
                }
            }
        }
    }

    if !reproduced_bug {
        println!("FAIL: the degenerate scroll-bar rect did not reproduce the panic");
        failures += 1;
    }

    if failures > 0 {
        std::process::exit(1);
    }
}
