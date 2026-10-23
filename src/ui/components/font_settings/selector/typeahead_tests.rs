use super::scroll_tests::{assert_visible, families, open};
use super::tests::{frame_with_families as frame, key};
use super::*;

#[test]
fn typing_jumps_to_a_visible_prefix_match_without_filtering_or_confirming() {
    let ctx = egui::Context::default();
    let families = families();
    open(&ctx, &families, "");
    let output = frame(
        &ctx,
        &families,
        "",
        vec![egui::Event::Text("TEST FONT 21".into())],
    );
    assert_eq!(output.preview.as_deref(), Some("Test Font 210"));
    assert_eq!(output.rows.len(), families.len() + 1);
    assert!(output.open);
    assert!(output.confirmed.is_none());
    let output = frame(&ctx, &families, "", vec![]);
    assert_visible(&output);
    let output = frame(&ctx, &families, "", key(egui::Key::Enter));
    assert_eq!(output.confirmed.as_deref(), Some("Test Font 210"));
    assert!(!output.open);
}

#[test]
fn incremental_typing_including_space_does_not_activate_a_focused_row() {
    let ctx = egui::Context::default();
    let families = families();
    open(&ctx, &families, "");
    frame(&ctx, &families, "", vec![egui::Event::Text("Test".into())]);
    let mut space = key(egui::Key::Space);
    space.push(egui::Event::Text(" Font ".into()));
    let output = frame(&ctx, &families, "", space);
    assert!(output.open);
    assert!(output.confirmed.is_none());
    let output = frame(&ctx, &families, "", vec![egui::Event::Text("179".into())]);
    assert_eq!(output.preview.as_deref(), Some("Test Font 179"));
    let output = frame(&ctx, &families, "", key(egui::Key::ArrowDown));
    assert_eq!(output.preview.as_deref(), Some("Test Font 180"));
    let output = frame(
        &ctx,
        &families,
        "",
        vec![egui::Event::Text("Test Font 002".into())],
    );
    assert_eq!(output.preview.as_deref(), Some("Test Font 002"));
}

#[test]
fn unmatched_names_do_not_change_selection_and_escape_clears_the_prefix() {
    let ctx = egui::Context::default();
    let families = families();
    let saved = &families[50];
    open(&ctx, &families, saved);
    let output = frame(
        &ctx,
        &families,
        saved,
        vec![egui::Event::Text("Missing Font".into())],
    );
    assert!(output.preview.is_none());
    assert_eq!(output.active, 51);
    let output = frame(&ctx, &families, saved, key(egui::Key::Escape));
    assert!(output.cancelled);
    assert!(output.confirmed.is_none());
    open(&ctx, &families, saved);
    let output = frame(
        &ctx,
        &families,
        saved,
        vec![egui::Event::Text("Test Font 002".into())],
    );
    assert_eq!(output.preview.as_deref(), Some("Test Font 002"));
}

#[test]
fn pausing_starts_a_new_name_and_backspace_edits_it() {
    let ctx = egui::Context::default();
    let families = families();
    open(&ctx, &families, "");
    frame(
        &ctx,
        &families,
        "",
        vec![egui::Event::Text("Test Font 100".into())],
    );
    for _ in 0..70 {
        frame(&ctx, &families, "", vec![]);
    }
    let output = frame(
        &ctx,
        &families,
        "",
        vec![egui::Event::Text("Test Font 200".into())],
    );
    assert_eq!(output.preview.as_deref(), Some("Test Font 200"));
    frame(&ctx, &families, "", key(egui::Key::Backspace));
    let output = frame(&ctx, &families, "", vec![egui::Event::Text("5".into())]);
    assert_eq!(output.preview.as_deref(), Some("Test Font 205"));
}

#[test]
fn non_ascii_font_names_match_case_insensitively() {
    let ctx = egui::Context::default();
    let families = vec!["Demo Font".into(), "Árvore Font".into()];
    open(&ctx, &families, "");
    let output = frame(&ctx, &families, "", vec![egui::Event::Text("ÁR".into())]);
    assert_eq!(output.preview.as_deref(), Some("Árvore Font"));
}

#[test]
fn text_and_backspace_in_the_same_frame_are_processed_in_input_order() {
    let ctx = egui::Context::default();
    let families = families();
    open(&ctx, &families, "");
    frame(
        &ctx,
        &families,
        "",
        vec![egui::Event::Text("Test Font 1".into())],
    );
    let mut events = vec![egui::Event::Text("2".into())];
    events.extend(key(egui::Key::Backspace));
    let output = frame(&ctx, &families, "", events);
    assert_eq!(output.active, 101);
    assert!(output.open);
}
