use super::*;
use crate::{FontChains, ImageMap, render_png};
use docx_layout::display_list::DisplayList;
use serde_json::{Value, json};

const CARLITO: &[u8] = include_bytes!("../../../tests/assets/Carlito-Regular.ttf");

fn fonts() -> (FontStore, FontId) {
    let mut store = FontStore::new();
    let id = store.register(CARLITO.to_vec()).unwrap();
    (store, id)
}

fn run(text: &str) -> TextRunPrimitive {
    serde_json::from_value(json!({
        "text": text, "x": 8, "baselineY": 40, "width": 230,
        "font": "400 32px Carlito, sans-serif", "color": "#000000",
        "smallCaps": true
    }))
    .unwrap()
}

fn scene(primitives: Vec<Value>) -> DisplayList {
    serde_json::from_value(json!({
        "pages": [{"pageIndex": 0, "width": 256, "height": 64, "primitives": primitives}]
    }))
    .unwrap()
}

fn render(store: &FontStore, chain: &[FontId], list: &DisplayList) -> Vec<u8> {
    let chains = FontChains::from([("carlito|0|0".to_owned(), chain.to_vec())]);
    let images = ImageMap::new();
    render_png(list, 0, &RenderResources::new(store, &chains, &images)).unwrap()
}

fn text_scene(run: &TextRunPrimitive) -> DisplayList {
    let mut value = serde_json::to_value(run).unwrap();
    value["kind"] = json!("text");
    scene(vec![value])
}

#[test]
fn synthesized_segments_keep_original_capitals_and_expand_lowercase() {
    let (store, id) = fonts();
    assert!(!supports_smcp(&store, id, 'e', 32.0, ShapeDirection::Ltr).unwrap());
    let parts = segments(&store, &[id], &run("Helloße"), 32.0, ShapeDirection::Ltr).unwrap();
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].text, "H");
    assert_eq!(parts[0].scale, 1.0);
    assert_eq!(parts[1].text, "ELLOSSE");
    assert_eq!(parts[1].scale, WORD_SMALL_CAPS_ADVANCE_SCALE);
    assert!(!parts[1].native);
    assert_eq!(parts[1].source(4), parts[1].source(5));
}

#[test]
fn language_and_all_caps_precedence_match_measurement() {
    let (store, id) = fonts();
    let mut input = run("iıß");
    input.attrs.lang = Some("tr-TR".into());
    let parts = segments(&store, &[id], &input, 32.0, ShapeDirection::Ltr).unwrap();
    assert_eq!(
        parts.iter().map(|s| s.text.as_str()).collect::<String>(),
        "İISS"
    );
    input.all_caps = true;
    let parts = segments(&store, &[id], &input, 32.0, ShapeDirection::Ltr).unwrap();
    assert!(parts.iter().all(|s| !s.native && s.scale == 1.0));
    assert_eq!(
        parts.iter().map(|s| s.text.as_str()).collect::<String>(),
        "İISS"
    );
}

#[test]
fn synthesized_ink_is_uppercase_at_eighty_percent_and_source_is_unchanged() {
    let (store, id) = fonts();
    let original = run("hello");
    let list = text_scene(&original);
    let before = list.clone();
    let actual = render(&store, &[id], &list);
    let mut expected = run("HELLO");
    expected.small_caps = false;
    expected.font = "400 25.6px Carlito, sans-serif".into();
    assert_eq!(actual, render(&store, &[id], &text_scene(&expected)));
    assert_eq!(list, before);
    let image = image::load_from_memory(&actual).unwrap().into_rgba8();
    assert!(image.pixels().any(|pixel| pixel[0] < 128 && pixel[3] > 0));
}

#[test]
fn shorthand_flag_and_explicit_flag_render_identically() {
    let (store, id) = fonts();
    let explicit = run("Hello");
    let mut shorthand = explicit.clone();
    shorthand.small_caps = false;
    shorthand.font = "small-caps 400 32px Carlito, sans-serif".into();
    assert_eq!(
        render(&store, &[id], &text_scene(&explicit)),
        render(&store, &[id], &text_scene(&shorthand)),
    );
}

#[test]
fn uppercase_expansion_has_no_internal_letter_spacing() {
    let (store, id) = fonts();
    let mut input = run("ß");
    input.letter_spacing = Some(12.into());
    let actual = render(&store, &[id], &text_scene(&input));
    input.letter_spacing = None;
    assert_eq!(actual, render(&store, &[id], &text_scene(&input)));
    assert_eq!(painted_chars(&input), 2);
}

#[test]
fn mixed_segments_render_with_whole_run_rotation_scale_clip_and_direction() {
    let (store, id) = fonts();
    for rtl in [false, true] {
        let mut input = run("Ab Cd");
        input.rtl = Some(rtl);
        input.rotation_deg = Some(8.into());
        input.horizontal_scale = Some(90.into());
        input.paint_clip = Some(serde_json::from_value(json!({"x": 8, "w": 160})).unwrap());
        input.letter_spacing = Some(2.into());
        input.word_spacing = Some(3.into());
        let actual = render(&store, &[id], &text_scene(&input));
        assert!(!actual.is_empty());
    }
}

#[test]
fn uncovered_characters_use_the_terminal_font_like_measurement() {
    let (mut store, primary) = fonts();
    let terminal = store.register(CARLITO.to_vec()).unwrap();
    let missing = '\u{10ffff}';
    assert!(!store.covers(primary, missing).unwrap());
    assert_eq!(
        resolve(&store, &[primary, terminal], missing).unwrap(),
        terminal
    );
    let parts = segments(
        &store,
        &[primary, terminal],
        &run("a\u{10ffff}B"),
        32.0,
        ShapeDirection::Ltr,
    )
    .unwrap();
    assert_eq!(parts.len(), 3);
    assert_eq!(parts[1].font, terminal);
    assert_eq!(parts[1].scale, 1.0);
    assert!(
        !render(
            &store,
            &[primary, terminal],
            &text_scene(&run("a\u{10ffff}B"))
        )
        .is_empty()
    );
}

#[test]
fn positioned_small_caps_glyphs_are_not_transformed_twice() {
    let (store, id) = fonts();
    let glyphs = shape(&store, id, "A", 24.0, &[]).unwrap();
    let mut value = json!({
        "kind": "glyphRun", "fontId": id.to_u32(), "size": 24,
        "color": "#000000", "text": "a", "smallCaps": true,
        "glyphs": [{"id": glyphs[0].glyph_id, "x": 8, "y": 40, "cluster": 0, "advance": 16}]
    });
    let actual = render(&store, &[id], &scene(vec![value.clone()]));
    value["smallCaps"] = json!(false);
    assert_eq!(actual, render(&store, &[id], &scene(vec![value])));
}

#[test]
fn uppercase_budget_is_enforced_before_allocating_segments() {
    let (store, id) = fonts();
    let input = run(&"ß".repeat(crate::MAX_PAGE_GLYPHS as usize / 2 + 1));
    let chains = FontChains::from([("carlito|0|0".to_owned(), vec![id])]);
    let images = ImageMap::new();
    let error = render_png(
        &text_scene(&input),
        0,
        &RenderResources::new(&store, &chains, &images),
    )
    .unwrap_err();
    assert!(error.contains("glyph"), "{error}");
}

#[cfg(target_os = "windows")]
#[test]
fn native_small_caps_use_the_fonts_substitution_at_full_size() {
    // The application's Windows font registry uses this same system face.
    let bytes = std::fs::read(
        std::path::PathBuf::from(std::env::var_os("WINDIR").unwrap())
            .join("Fonts")
            .join("calibri.ttf"),
    )
    .unwrap();
    let mut store = FontStore::new();
    let id = store.register(bytes).unwrap();
    let input = run("abc");
    let parts = segments(&store, &[id], &input, 32.0, ShapeDirection::Ltr).unwrap();
    assert!(parts.iter().all(|s| s.native && s.scale == 1.0));
    let shaped =
        shape_with_properties(&store, id, "abc", 32.0, &SMCP, ShapeDirection::Ltr, None).unwrap();
    let mut pen = 8.0;
    let glyphs: Vec<_> = shaped
        .iter()
        .map(|g| {
            let placed = json!({
                "id": g.glyph_id, "x": pen + g.x_offset, "y": 40.0 - g.y_offset,
                "cluster": g.cluster, "advance": g.x_advance
            });
            pen += g.x_advance;
            placed
        })
        .collect();
    let expected = scene(vec![json!({
        "kind": "glyphRun", "fontId": id.to_u32(), "size": 32,
        "color": "#000000", "text": "abc", "glyphs": glyphs
    })]);
    assert_eq!(
        render(&store, &[id], &text_scene(&input)),
        render(&store, &[id], &expected),
    );
}
