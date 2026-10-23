use super::*;
use crate::infrastructure::app_state_db::AppStateDb;

fn fixture() -> (egui::Context, AppStateDb) {
    let db = AppStateDb::new_in_memory().unwrap();
    db.set_preference(PREFERENCE_KEY, "Test Font A").unwrap();
    db.set_preference(&sizing::preference_key("Test Font A"), "1.25")
        .unwrap();
    db.set_preference(&sizing::preference_key("Test Font B"), "0.85")
        .unwrap();
    let ctx = egui::Context::default();
    install(
        &ctx,
        LoadedFonts::with_catalog(
            egui::FontDefinitions::default(),
            "Test Font A".into(),
            Arc::new(Catalog::from_database(fontdb::Database::new())),
        ),
        FontSizes::load(&db),
    );
    (ctx, db)
}

fn width(ctx: &egui::Context) -> f32 {
    flush_font_updates(ctx);
    let mut width = 0.0;
    let _ = ctx.run_ui(Default::default(), |ui| {
        width = ui.label("Sample text 012345").rect.width();
    });
    width
}

fn complete(ctx: &egui::Context, family: &str, commit: bool) {
    let (sender, receiver) = mpsc::channel();
    sender.send(Some(egui::FontDefinitions::default())).unwrap();
    let state = state(ctx).unwrap();
    let mut state = state.lock().unwrap();
    state.pending = Some(Pending {
        selected: family.into(),
        receiver,
    });
    state.desired = Some(Request {
        selected: family.into(),
        commit,
    });
}

#[test]
fn startup_and_preview_use_each_familys_saved_size_and_cancel_restores_it() {
    let (ctx, db) = fixture();
    let original = width(&ctx);
    assert_eq!(snapshot(&ctx).unwrap().scale, 1.25);
    preview_frame(&ctx);
    complete(&ctx, "Test Font B", false);
    poll(&ctx, &db);
    assert!(width(&ctx) < original * 0.8);
    cancel_preview(&ctx);
    assert!((width(&ctx) - original).abs() < 0.1);
    assert_eq!(
        db.get_preference(PREFERENCE_KEY).as_deref(),
        Some("Test Font A")
    );
    assert_eq!(snapshot(&ctx).unwrap().scale, 1.25);
}

#[test]
fn switching_fonts_and_resetting_remembers_sizes_independently() {
    let (ctx, db) = fixture();
    complete(&ctx, "Test Font B", true);
    poll(&ctx, &db);
    assert_eq!(snapshot(&ctx).unwrap().scale, 0.85);
    set_scale(&ctx, &db, 1.4);
    assert_eq!(
        db.get_preference(&sizing::preference_key("Test Font B"))
            .as_deref(),
        Some("1.4")
    );
    set_scale(&ctx, &db, sizing::DEFAULT);
    complete(&ctx, "Test Font A", true);
    poll(&ctx, &db);
    assert_eq!(snapshot(&ctx).unwrap().scale, 1.25);
    complete(&ctx, "Test Font B", true);
    poll(&ctx, &db);
    assert_eq!(snapshot(&ctx).unwrap().scale, sizing::DEFAULT);
    let reloaded = FontSizes::load(&db);
    assert_eq!(reloaded.get("Test Font A"), 1.25);
    assert_eq!(reloaded.get("Test Font B"), sizing::DEFAULT);
}

#[test]
fn repeated_size_changes_do_not_compound_and_preserve_interface_zoom() {
    let (ctx, db) = fixture();
    crate::ui::interface_scale::apply(&ctx, 1.5);
    let original = width(&ctx);
    set_scale(&ctx, &db, 1.1);
    set_scale(&ctx, &db, 1.25);
    assert!((width(&ctx) - original).abs() < 0.1);
    assert_eq!(ctx.zoom_factor(), 1.5);
    set_scale(&ctx, &db, 1.1);
    assert!(width(&ctx) < original);
    set_scale(&ctx, &db, 1.25);
    assert!((width(&ctx) - original).abs() < 0.1);
}

#[test]
fn cancellation_in_the_same_pass_as_a_ready_preview_keeps_the_original_size() {
    let (ctx, db) = fixture();
    let original = width(&ctx);
    preview_frame(&ctx);
    complete(&ctx, "Test Font B", false);
    poll(&ctx, &db);
    assert_eq!(snapshot(&ctx).unwrap().scale, 0.85);
    cancel_preview(&ctx);
    assert!((width(&ctx) - original).abs() < 0.1);
    assert_eq!(snapshot(&ctx).unwrap().scale, 1.25);
}

#[test]
fn preview_does_not_allow_accidental_edits_to_the_committed_familys_size() {
    let (ctx, db) = fixture();
    preview_frame(&ctx);
    set_scale(&ctx, &db, 1.5);
    assert_eq!(snapshot(&ctx).unwrap().scale, 1.25);
    assert_eq!(
        db.get_preference(&sizing::preference_key("Test Font A"))
            .as_deref(),
        Some("1.25")
    );
}
