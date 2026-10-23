use super::*;
use crate::infrastructure::app_state_db::AppStateDb;

fn fixture(selected: &str) -> (egui::Context, AppStateDb) {
    let ctx = egui::Context::default();
    let db = AppStateDb::new_in_memory().unwrap();
    db.set_preference(PREFERENCE_KEY, selected).unwrap();
    let base = egui::FontDefinitions::default();
    install(
        &ctx,
        LoadedFonts::with_catalog(
            base,
            selected.into(),
            Arc::new(Catalog::from_database(fontdb::Database::new())),
        ),
    );
    (ctx, db)
}

fn complete(ctx: &egui::Context, selected: &str, result: Option<egui::FontDefinitions>) {
    let (sender, receiver) = mpsc::channel();
    sender.send(result).unwrap();
    let state = state(ctx).unwrap();
    let mut state = state.lock().unwrap();
    state.desired = Some(Request {
        selected: selected.into(),
        commit: true,
    });
    state.pending = Some(Pending {
        selected: selected.into(),
        receiver,
    });
}

#[test]
fn saved_family_is_loaded_again_at_startup() {
    let base = egui::FontDefinitions::default();
    let mut database = fontdb::Database::new();
    for font in base.font_data.values() {
        database.load_font_data(font.font.to_vec());
    }
    let catalog = Arc::new(Catalog::from_database(database));
    let selected = catalog.families[0].clone();
    let db = AppStateDb::new_in_memory().unwrap();
    db.set_preference(PREFERENCE_KEY, &selected).unwrap();
    let loaded =
        LoadedFonts::with_catalog(base, db.get_preference(PREFERENCE_KEY).unwrap(), catalog);
    assert_eq!(loaded.selected, selected);
    assert!(!loaded.unavailable);
    assert_eq!(
        loaded.fonts.families[&egui::FontFamily::Proportional][0],
        super::super::catalog::CUSTOM_FONT
    );
    let ctx = egui::Context::default();
    install(&ctx, loaded);
    let _ = ctx.run_ui(Default::default(), |ui| {
        ui.label("Sample text");
    });
    assert_eq!(snapshot(&ctx).unwrap().selected, selected);
}

#[test]
fn unavailable_saved_font_keeps_default_and_allows_reset() {
    let (ctx, db) = fixture("Missing Test Family");
    assert!(snapshot(&ctx).unwrap().unavailable);
    let base = state(&ctx).unwrap().lock().unwrap().base.clone();
    complete(&ctx, "", Some(base));
    poll(&ctx, &db);
    assert_eq!(db.get_preference(PREFERENCE_KEY).as_deref(), Some(""));
    let snapshot = snapshot(&ctx).unwrap();
    assert_eq!(snapshot.selected, "");
    assert!(!snapshot.unavailable);
    assert!(!snapshot.loading);
}

#[test]
fn failed_switch_preserves_selected_font_and_saved_preference() {
    let (ctx, db) = fixture("");
    complete(&ctx, "Missing Test Family", None);
    poll(&ctx, &db);
    assert_eq!(snapshot(&ctx).unwrap().selected, "");
    assert!(snapshot(&ctx).unwrap().unavailable);
    assert_eq!(db.get_preference(PREFERENCE_KEY).as_deref(), Some(""));
}

#[test]
fn successful_switch_is_saved_without_changing_interface_scale() {
    let (ctx, db) = fixture("");
    crate::ui::interface_scale::apply(&ctx, 1.5);
    let _ = ctx.run_ui(Default::default(), |_| {});
    assert_eq!(ctx.zoom_factor(), 1.5);
    complete(&ctx, "Test Family", Some(egui::FontDefinitions::default()));
    poll(&ctx, &db);
    let _ = ctx.run_ui(Default::default(), |_| {});
    assert_eq!(snapshot(&ctx).unwrap().selected, "Test Family");
    assert_eq!(
        db.get_preference(PREFERENCE_KEY).as_deref(),
        Some("Test Family")
    );
    assert_eq!(ctx.zoom_factor(), 1.5);
    assert!(!snapshot(&ctx).unwrap().save_failed);
}

#[test]
fn disconnected_loader_does_not_stay_pending_or_change_preference() {
    let (ctx, db) = fixture("");
    let (sender, receiver) = mpsc::channel();
    drop(sender);
    state(&ctx).unwrap().lock().unwrap().pending = Some(Pending {
        selected: "Test Family".into(),
        receiver,
    });
    state(&ctx).unwrap().lock().unwrap().desired = Some(Request {
        selected: "Test Family".into(),
        commit: true,
    });
    poll(&ctx, &db);
    assert!(!snapshot(&ctx).unwrap().loading);
    assert!(snapshot(&ctx).unwrap().unavailable);
    assert_eq!(db.get_preference(PREFERENCE_KEY).as_deref(), Some(""));
}

#[test]
fn metrics_are_invalidated_on_the_pass_after_fonts_take_effect() {
    let (ctx, db) = fixture("");
    assert!(state(&ctx)
        .unwrap()
        .lock()
        .unwrap()
        .metrics_changed_at
        .is_some());
    let _ = ctx.run_ui(Default::default(), |_| {});
    poll(&ctx, &db);
    assert!(state(&ctx)
        .unwrap()
        .lock()
        .unwrap()
        .metrics_changed_at
        .is_none());
}

fn complete_preview(ctx: &egui::Context, selected: &str, fonts: egui::FontDefinitions) {
    preview_frame(ctx);
    complete(ctx, selected, Some(fonts));
    state(ctx)
        .unwrap()
        .lock()
        .unwrap()
        .desired
        .as_mut()
        .unwrap()
        .commit = false;
}

#[test]
fn preview_changes_rendered_text_but_cancel_restores_without_saving() {
    fn width(ctx: &egui::Context) -> f32 {
        let mut width = 0.0;
        let _ = ctx.run_ui(Default::default(), |ui| {
            width = ui.label("iiiiiiiiiiiiiiii").rect.width();
        });
        width
    }
    let (ctx, db) = fixture("");
    let original_width = width(&ctx);
    let mut fonts = egui::FontDefinitions::default();
    fonts.families.insert(
        egui::FontFamily::Proportional,
        fonts.families[&egui::FontFamily::Monospace].clone(),
    );
    complete_preview(&ctx, "Test Family", fonts);
    poll(&ctx, &db);
    assert!((width(&ctx) - original_width).abs() > 1.0);
    assert_eq!(snapshot(&ctx).unwrap().selected, "");
    assert_eq!(db.get_preference(PREFERENCE_KEY).as_deref(), Some(""));
    cancel_preview(&ctx);
    assert!((width(&ctx) - original_width).abs() < 0.1);
    assert_eq!(db.get_preference(PREFERENCE_KEY).as_deref(), Some(""));
}

#[test]
fn confirmation_promotes_inflight_preview_and_saves_only_confirmed_family() {
    let (ctx, db) = fixture("");
    complete_preview(&ctx, "Test Family", egui::FontDefinitions::default());
    assert!(!snapshot(&ctx).unwrap().committing);
    request(&ctx, "Test Family".into());
    assert!(snapshot(&ctx).unwrap().committing);
    poll(&ctx, &db);
    assert_eq!(snapshot(&ctx).unwrap().selected, "Test Family");
    assert_eq!(
        db.get_preference(PREFERENCE_KEY).as_deref(),
        Some("Test Family")
    );
    cancel_preview(&ctx);
    assert_eq!(snapshot(&ctx).unwrap().selected, "Test Family");
}

#[test]
fn late_preview_result_cannot_override_cancellation() {
    let (ctx, db) = fixture("");
    complete_preview(&ctx, "Test Family", egui::FontDefinitions::default());
    cancel_preview(&ctx);
    poll(&ctx, &db);
    let state = state(&ctx).unwrap();
    let state = state.lock().unwrap();
    assert!(state.pending.is_none());
    assert!(state.desired.is_none());
    assert!(state.preview_pass.is_none());
    assert_eq!(state.selected, "");
    assert_eq!(db.get_preference(PREFERENCE_KEY).as_deref(), Some(""));
}

#[test]
fn fast_navigation_coalesces_requests_and_ignores_outdated_results() {
    let (ctx, db) = fixture("");
    complete_preview(&ctx, "Test Family A", egui::FontDefinitions::default());
    preview(&ctx, "Test Family B".into());
    preview(&ctx, "Test Family C".into());
    poll(&ctx, &db);
    let state = state(&ctx).unwrap();
    let state = state.lock().unwrap();
    assert_eq!(state.pending.as_ref().unwrap().selected, "Test Family C");
    assert_eq!(state.selected, "");
    assert_eq!(db.get_preference(PREFERENCE_KEY).as_deref(), Some(""));
}

#[test]
fn removing_the_selector_cancels_its_preview() {
    let (ctx, db) = fixture("");
    complete_preview(&ctx, "Test Family", egui::FontDefinitions::default());
    let _ = ctx.run_ui(Default::default(), |_| {});
    finish_preview_frame(&ctx);
    poll(&ctx, &db);
    assert!(state(&ctx).unwrap().lock().unwrap().preview_pass.is_none());
    assert_eq!(db.get_preference(PREFERENCE_KEY).as_deref(), Some(""));
}
