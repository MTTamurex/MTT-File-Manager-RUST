use super::*;

fn texts(output: &egui::FullOutput) -> Vec<String> {
    output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
            _ => None,
        })
        .collect()
}

#[test]
fn technical_details_are_collapsed_by_default() {
    let ctx = egui::Context::default();
    let stats = DuplicateStats {
        processed_files: 100,
        total_candidates: 100,
        hashed_bytes: 1024,
        partial_rejected: 50,
        full_candidates: 50,
        skipped_by_limit: 1_000_000,
        ..Default::default()
    };
    let output = ctx.run_ui(egui::RawInput::default(), |ui| {
        render(ui, &stats, None, true)
    });
    let labels = texts(&output);
    assert!(labels
        .iter()
        .any(|s| s == &t!("disk_analysis.dup_search_details").to_string()));
    assert!(!labels.iter().any(|s| s
        == &t!(
            "disk_analysis.dup_report_stats",
            processed = 100,
            candidates = 100,
            bytes = format_size(1024)
        )
        .to_string()));
    assert!(!labels
        .iter()
        .any(|s| s == &t!("disk_analysis.dup_skipped_limit", count = 1_000_000).to_string()));
}

#[test]
fn incomplete_warning_stays_visible_but_reasons_are_collapsed() {
    let ctx = egui::Context::default();
    let output = ctx.run_ui(egui::RawInput::default(), |ui| warnings(ui, 606, 5, 2));
    let labels = texts(&output);
    assert!(labels
        .iter()
        .any(|s| s == &t!("disk_analysis.dup_unchecked_warning", count = 613).to_string()));
    assert!(!labels
        .iter()
        .any(|s| s == &t!("disk_analysis.dup_inaccessible", count = 606).to_string()));
    let output = ctx.run_ui(egui::RawInput::default(), |ui| warnings(ui, 0, 0, 0));
    assert!(texts(&output).is_empty());
}

#[test]
fn intentional_size_exclusions_are_not_counted_as_verification_failures() {
    assert_eq!(unchecked(606, 5, 0), 611);
    assert_eq!(unchecked(0, 0, 0), 0);
    assert_eq!(unchecked(u64::MAX, 1, 1), u64::MAX);
}

#[test]
fn expanding_search_details_reveals_the_preserved_counters() {
    let ctx = egui::Context::default();
    let stats = DuplicateStats {
        processed_files: 100,
        total_candidates: 100,
        hashed_bytes: 1024,
        ..Default::default()
    };
    let frame = |time, events| {
        ctx.run_ui(
            egui::RawInput {
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| render(ui, &stats, None, true),
        )
    };
    let first = frame(0.0, Vec::new());
    let header = t!("disk_analysis.dup_search_details").to_string();
    let target = first
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == header => {
                Some(text.pos + text.galley.size() * 0.5)
            }
            _ => None,
        })
        .expect("details header");
    let _ = frame(
        0.1,
        vec![
            egui::Event::PointerMoved(target),
            egui::Event::PointerButton {
                pos: target,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    let _ = frame(
        0.2,
        vec![egui::Event::PointerButton {
            pos: target,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    let expanded = frame(0.6, Vec::new());
    let expected = t!(
        "disk_analysis.dup_report_stats",
        processed = 100,
        candidates = 100,
        bytes = format_size(1024)
    )
    .to_string();
    assert!(texts(&expanded).contains(&expected));
}
