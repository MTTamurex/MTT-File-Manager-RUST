use super::*;
use crate::infrastructure::copy_engine::CopyJob;
use crate::infrastructure::delete_engine::confirmation::{
    ask, tests::wait_for_request, Kind, Request,
};

struct Text {
    value: String,
    rect: egui::Rect,
    clip: egui::Rect,
}

fn collect(shape: &egui::epaint::Shape, clip: egui::Rect, texts: &mut Vec<Text>) {
    match shape {
        egui::epaint::Shape::Text(text) => texts.push(Text {
            value: text.galley.job.text.clone(),
            rect: text.galley.rect.translate(text.pos.to_vec2()),
            clip,
        }),
        egui::epaint::Shape::Vec(shapes) => {
            for shape in shapes {
                collect(shape, clip, texts);
            }
        }
        _ => {}
    }
}

struct Fixture {
    ctx: egui::Context,
    runtime: CopyRuntime,
    panel: CopyPanelState,
    job: CopyJob,
    request: Request,
    worker: Option<std::thread::JoinHandle<bool>>,
    frame: u64,
}

impl Fixture {
    fn new(scale: f32, count: usize) -> Self {
        let ctx = egui::Context::default();
        crate::ui::interface_scale::apply(&ctx, scale);
        ctx.global_style_mut(|style| {
            style.interaction.tooltip_delay = 0.0;
            style.interaction.show_tooltips_only_when_still = false;
        });
        let runtime = CopyRuntime::default();
        let paths = (0..count)
            .map(|index| PathBuf::from(format!("source/nested/file-{index:04}.txt")))
            .collect();
        let job = runtime.start_delete(paths, false);
        let operation = job.operation.clone();
        let worker = std::thread::spawn(move || ask(&operation, Kind::Recycle, &operation.sources));
        let request = wait_for_request(&job.operation);
        Self {
            ctx,
            runtime,
            panel: CopyPanelState::default(),
            job,
            request,
            worker: Some(worker),
            frame: 0,
        }
    }

    fn draw(&mut self, events: Vec<egui::Event>) -> Vec<Text> {
        self.frame += 1;
        let jobs = self.runtime.jobs();
        let panel = &mut self.panel;
        let output = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 1000.0),
                )),
                time: Some(self.frame as f64 / 60.0),
                events,
                ..Default::default()
            },
            |ui| {
                super::super::super::window::show(ui.ctx(), &jobs, panel);
            },
        );
        let mut texts = Vec::new();
        for shape in output.shapes {
            collect(&shape.shape, shape.clip_rect, &mut texts);
        }
        texts
    }

    fn settle(&mut self) -> Vec<Text> {
        for _ in 0..8 {
            self.draw(vec![]);
        }
        self.draw(vec![])
    }

    fn size(&self) -> egui::Vec2 {
        self.ctx
            .memory(|memory| memory.area_rect(egui::Id::new("mtt_copy_progress_panel")))
            .unwrap()
            .size()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.job.operation.progress.cancel();
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}

#[test]
fn confirmation_has_one_question_and_no_progress_or_redundant_summaries() {
    for scale in [1.0, 2.0] {
        let mut fixture = Fixture::new(scale, 5);
        let texts = fixture.settle();
        let question = t!("delete_engine.recycle_confirm", count = 5);
        assert_eq!(
            texts.iter().filter(|text| text.value == question).count(),
            1
        );
        assert_eq!(
            texts
                .iter()
                .filter(|text| text.value == t!("delete_engine.awaiting_confirmation"))
                .count(),
            1
        );
        for label in [
            t!("copy_engine.elapsed"),
            t!("delete_engine.removed"),
            t!("copy_engine.source"),
            t!("delete_engine.mode"),
        ] {
            assert!(!texts.iter().any(|text| text.value == label));
        }
        assert!(!texts
            .iter()
            .any(|text| text.value.contains("source/nested")));
        assert!(!texts
            .iter()
            .any(|text| text.value == t!("delete_engine.apply_to_remaining")));
        let question = texts.iter().find(|text| text.value == question).unwrap();
        let first = texts
            .iter()
            .find(|text| text.value == "file-0000.txt")
            .unwrap();
        let cancel = texts
            .iter()
            .find(|text| text.value == t!("copy_engine.cancel"))
            .unwrap();
        let last = texts
            .iter()
            .find(|text| text.value == "file-0004.txt")
            .unwrap();
        assert!(first.rect.top() - question.rect.bottom() >= 10.0);
        assert!(
            (0.0..=16.0).contains(&(first.rect.left() - question.rect.left())),
            "Filenames must be left-aligned with the question"
        );
        assert!(cancel.rect.top() - last.rect.bottom() >= 10.0);
        for control in [
            t!("copy_engine.cancel"),
            t!("delete_engine.confirm_recycle"),
        ] {
            let text = texts.iter().find(|text| text.value == control).unwrap();
            assert!(
                text.clip.contains_rect(text.rect),
                "Clipped confirmation button at scale {scale}"
            );
        }

        fixture
            .job
            .operation
            .deletion
            .as_ref()
            .unwrap()
            .confirmation
            .respond(fixture.request.id, true);
        assert!(fixture.worker.take().unwrap().join().unwrap());
        let texts = fixture.settle();
        assert!(texts
            .iter()
            .any(|text| text.value == t!("copy_engine.elapsed")));
        assert!(texts
            .iter()
            .any(|text| text.value == t!("delete_engine.removed")));
        assert!(!texts
            .iter()
            .any(|text| text.value == t!("delete_engine.confirm_recycle")));
    }
}

#[test]
fn large_selection_keeps_five_rows_and_can_scroll_to_the_last_item() {
    for scale in [1.0, 2.0] {
        let mut small = Fixture::new(scale, 5);
        small.settle();
        let small_size = small.size();
        let mut large = Fixture::new(scale, 1000);
        let texts = large.settle();
        let size = large.size();
        assert!(
            (size.y - small_size.y).abs() <= 1.0,
            "Selection resized the window: {small_size:?} -> {size:?}"
        );
        let rows: Vec<_> = texts
            .iter()
            .filter(|text| text.value.starts_with("file-") && text.clip.contains_rect(text.rect))
            .collect();
        assert_eq!(
            rows.len(),
            5,
            "Visible rows at scale {scale}: {:?}",
            texts
                .iter()
                .filter(|text| text.value.starts_with("file-"))
                .map(|text| (&text.value, text.rect, text.clip))
                .collect::<Vec<_>>()
        );
        assert!(!texts.iter().any(|text| text.value == "file-0999.txt"));
        let pointer = rows[2].rect.center();
        large.draw(vec![
            egui::Event::PointerMoved(pointer),
            egui::Event::MouseWheel {
                phase: egui::TouchPhase::Move,
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -1_000_000.0),
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        let texts = large.settle();
        assert!(texts
            .iter()
            .any(|text| text.value == "file-0999.txt" && text.clip.contains_rect(text.rect)));
        assert!((large.size().y - size.y).abs() <= 1.0);
    }
}

#[test]
fn filename_tooltip_exposes_the_full_path_without_filling_the_list_with_paths() {
    let mut fixture = Fixture::new(1.0, 1);
    let texts = fixture.settle();
    let row = texts
        .iter()
        .find(|text| text.value == "file-0000.txt")
        .unwrap();
    fixture.draw(vec![egui::Event::PointerMoved(row.rect.center())]);
    let texts = fixture.settle();
    assert!(texts
        .iter()
        .any(|text| text.value == "source/nested/file-0000.txt"));
}
