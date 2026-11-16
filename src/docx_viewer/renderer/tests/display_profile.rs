use super::*;
use docx_edit::EngineStats;

struct Frame {
    bytes: Vec<u8>,
    list: DisplayList,
    stats: EngineStats,
    marks: usize,
}

fn build(bytes: &[u8], observed: bool) -> Frame {
    let parsed =
        parse_docx_s9_wire_with_limits(bytes, S9ParseOptions::default(), &ParseLimits::default())
            .unwrap();
    let engine = EngineSession::new(0x4d54_444f_4358);
    seed_from_docx(engine.doc(), bytes).unwrap();
    let probe = region_request(&parsed.document.package, None).unwrap();
    let requirements: Value = serde_json::from_str(
        &engine
            .layout_font_requirements_json(&probe.to_string())
            .unwrap(),
    )
    .unwrap();
    let fonts = RegisteredFonts::load(&requirements).unwrap();
    let request = region_request(&parsed.document.package, Some(&fonts.layout_chain_ids)).unwrap();
    let reply = engine
        .layout_document_with_regions_retained_json(&request.to_string())
        .unwrap();
    let extras = display_extras(&reply, &fonts.layout_chain_ids).unwrap();
    let mut marks = 0;
    let bytes = if observed {
        engine
            .build_display_list_frame_observed(&extras, 0, &mut || marks += 1)
            .unwrap()
    } else {
        engine.build_display_list_frame(&extras, 0).unwrap()
    };
    Frame {
        bytes,
        list: engine.with_display_list(Clone::clone).unwrap(),
        stats: engine.stats(),
        marks,
    }
}

#[test]
fn observation_preserves_binary_frame_list_and_engine_state() {
    for bytes in [synthetic_docx(), regions::region_docx(true, true)] {
        let ordinary = build(&bytes, false);
        let observed = build(&bytes, true);
        assert!(!ordinary.bytes.is_empty());
        assert_eq!(observed.list, ordinary.list);
        assert_eq!(observed.bytes, ordinary.bytes);
        assert_eq!(
            observed.marks, 3,
            "the opening must be a full three-phase build"
        );
        assert_eq!(ordinary.marks, 0);
        assert_eq!(observed.stats.display_builds, 1);
        assert_eq!(observed.stats.display_builds, ordinary.stats.display_builds);
        assert_eq!(observed.stats.frame_epoch, ordinary.stats.frame_epoch);
        assert_eq!(observed.stats.layout_epoch, ordinary.stats.layout_epoch);
        assert_eq!(
            observed.stats.pagination_calls,
            ordinary.stats.pagination_calls
        );
        assert_eq!(
            observed.stats.retained_display_pages,
            ordinary.stats.retained_display_pages
        );
        assert_eq!(
            observed.stats.retained_display_primitives,
            ordinary.stats.retained_display_primitives,
        );
    }
}

#[test]
fn observation_preserves_errors_without_reporting_unfinished_phases() {
    let engine = EngineSession::new(0x4d54_444f_4358);
    let ordinary = engine.build_display_list_frame("{}", 0).unwrap_err();
    let mut marks = 0;
    let observed = engine
        .build_display_list_frame_observed("{}", 0, &mut || marks += 1)
        .unwrap_err();
    assert_eq!(observed, ordinary);
    assert_eq!(marks, 0);
    assert_eq!(engine.stats().display_builds, 0);
    assert_eq!(
        display_build::build_frame(&engine, "{}", OpenTrace::default()).unwrap_err(),
        ordinary,
    );
}
