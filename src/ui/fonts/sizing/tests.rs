use super::*;
use crate::infrastructure::app_state_db::AppStateDb;

#[test]
fn scales_are_validated_and_default_font_alias_is_shared() {
    for (value, expected) in [
        (None, 1.0),
        (Some("bad"), 1.0),
        (Some("NaN"), 1.0),
        (Some("inf"), 1.0),
        (Some("0.2"), MIN),
        (Some("3"), MAX),
        (Some("1.25"), 1.25),
    ] {
        assert_eq!(from_preference(value), expected);
    }
    assert_eq!(preference_key(""), preference_key("Segoe UI"));
    assert_eq!(preference_key("Test Font"), preference_key("TEST FONT"));
}

#[test]
fn saved_sizes_load_independently_by_family() {
    let db = AppStateDb::new_in_memory().unwrap();
    db.set_preference(&preference_key("Test Font A"), "1.25")
        .unwrap();
    db.set_preference(&preference_key("Test Font B"), "0.85")
        .unwrap();
    db.set_preference(&preference_key("Test Font C"), "NaN")
        .unwrap();
    let sizes = FontSizes::load(&db);
    assert_eq!(sizes.get("Test Font A"), 1.25);
    assert_eq!(sizes.get("Test Font B"), 0.85);
    assert_eq!(sizes.get("Test Font C"), DEFAULT);
    assert_eq!(sizes.get("Test Font D"), DEFAULT);
}

fn sample_metrics(fonts: egui::FontDefinitions) -> (f32, f32) {
    let ctx = egui::Context::default();
    ctx.set_fonts(fonts);
    let mut result = (0.0, 0.0);
    let _ = ctx.run_ui(Default::default(), |ui| {
        let label = ui.label("Sample text 012345");
        result = (label.rect.width(), label.rect.height());
    });
    result
}

#[test]
fn scaling_changes_real_layout_metrics_and_preserves_icon_and_monospace_data() {
    let mut fonts = egui::FontDefinitions::default();
    let icons = egui::FontFamily::Name("icons".into());
    fonts.families.insert(
        icons.clone(),
        fonts.families[&egui::FontFamily::Proportional].clone(),
    );
    let original = fonts.clone();
    let before = sample_metrics(fonts.clone());
    apply(&mut fonts, MAX);
    let after = sample_metrics(fonts.clone());
    assert!(after.0 > before.0 * 1.35);
    assert!(after.1 > before.1 * 1.35);
    assert_eq!(fonts.families[&icons], original.families[&icons]);
    assert_eq!(
        fonts.families[&egui::FontFamily::Monospace],
        original.families[&egui::FontFamily::Monospace]
    );
    for name in original.font_data.keys() {
        assert_eq!(
            fonts.font_data[name].tweak.scale,
            original.font_data[name].tweak.scale
        );
    }
}

#[test]
fn manual_rows_reserve_space_for_the_active_scaled_font() {
    let mut fonts = egui::FontDefinitions::default();
    apply(&mut fonts, MAX);
    let ctx = egui::Context::default();
    ctx.set_fonts(fonts);
    let _ = ctx.run_ui(Default::default(), |ui| {
        let line = ui.fonts_mut(|fonts| fonts.row_height(&egui::FontId::proportional(18.0)));
        let row = crate::ui::fonts::layout::row_height(ui, 24.0, 18.0, 6.0);
        assert!(row >= line + 6.0);
    });
}
