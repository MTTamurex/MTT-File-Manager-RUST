use super::*;
use crate::infrastructure::delete_engine::confirmation::{ask, tests::wait_for_request, Kind};

fn center(shape: &egui::epaint::Shape, label: &str) -> Option<egui::Pos2> {
    match shape {
        egui::epaint::Shape::Text(text) if text.galley.job.text == label => {
            Some(text.galley.rect.translate(text.pos.to_vec2()).center())
        }
        egui::epaint::Shape::Vec(shapes) => {
            shapes.iter().rev().find_map(|shape| center(shape, label))
        }
        _ => None,
    }
}

fn run_case(scale: f32, apply_to_all: bool, allow: bool) {
    let ctx = egui::Context::default();
    crate::ui::interface_scale::apply(&ctx, scale);
    let runtime = CopyRuntime::default();
    let job = runtime.start_delete(vec![PathBuf::from("source/file.txt")], false);
    let operation = job.operation.clone();
    let (sender, receiver) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        for _ in 0..2 {
            let answer = ask(&operation, Kind::RecycleUnavailable, &operation.sources);
            let _ = sender.send(answer);
        }
    });
    let request = wait_for_request(&job.operation);
    let mut panel = CopyPanelState::default();
    let mut draw = |events| {
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 1000.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                super::super::super::window::show(ui.ctx(), &runtime.jobs(), &mut panel);
            },
        )
    };
    for _ in 0..8 {
        draw(vec![]);
    }
    let point = |output: &egui::FullOutput, label: &str| {
        output
            .shapes
            .iter()
            .rev()
            .find_map(|shape| {
                center(&shape.shape, label).filter(|pos| shape.clip_rect.contains(*pos))
            })
            .expect("Confirmation control must be visible")
    };
    let click = |position, pressed| {
        vec![
            egui::Event::PointerMoved(position),
            egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    };
    let output = draw(vec![]);
    let checkbox = point(&output, &t!("delete_engine.apply_to_remaining"));
    if apply_to_all {
        draw(click(checkbox, true));
        draw(click(checkbox, false));
    }
    let output = draw(vec![]);
    let action = point(
        &output,
        &if allow {
            t!("delete_engine.confirm_permanent")
        } else {
            t!("copy_engine.cancel")
        },
    );
    if allow {
        draw(click(action, true));
        draw(click(action, false));
    } else {
        // Esc must still cancel when focus is on the newly added checkbox.
        draw(vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]);
    }
    let first = receiver.recv_timeout(std::time::Duration::from_secs(3));
    if first.is_err() {
        job.operation.progress.cancel();
    }
    if allow && !apply_to_all && first == Ok(true) {
        let next = wait_for_request(&job.operation);
        assert_ne!(
            request.id, next.id,
            "Unchecked scope must require a new confirmation"
        );
        job.operation
            .deletion
            .as_ref()
            .unwrap()
            .confirmation
            .respond(next.id, false);
    }
    let second = receiver.recv_timeout(std::time::Duration::from_secs(3));
    if second.is_err() {
        job.operation.progress.cancel();
    }
    worker.join().unwrap();
    assert_eq!(
        first,
        Ok(allow),
        "First answer: scale={scale}, scope={apply_to_all}, allow={allow}"
    );
    assert_eq!(
        second,
        Ok(allow && apply_to_all),
        "Second answer: scale={scale}, scope={apply_to_all}, allow={allow}"
    );
    assert_eq!(job.operation.progress.is_cancelled(), !allow);
}

#[test]
fn fallback_checkbox_is_unchecked_by_default_and_requires_an_explicit_click() {
    for scale in [1.0, 2.0] {
        run_case(scale, false, true);
        run_case(scale, true, true);
        run_case(scale, true, false);
    }
}
