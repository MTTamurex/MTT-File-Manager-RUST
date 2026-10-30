use super::*;

fn measurement(id: ColumnId, natural: f32, content: f32) -> ColumnMeasurement {
    ColumnMeasurement {
        id,
        width: natural,
        content,
    }
}

/// Name 300, Date 112 (content 96: a full "2025-07-24 07:56"), Type 64, Size 71.
fn list() -> Vec<ColumnMeasurement> {
    vec![
        measurement(ColumnId::Name, 300.0, 270.0),
        measurement(ColumnId::Date, 112.0, 96.0),
        measurement(ColumnId::Type, 64.0, 38.0),
        measurement(ColumnId::Size, 71.0, 55.0),
    ]
}

fn width_of(widths: &[(ColumnId, f32)], id: ColumnId) -> f32 {
    widths
        .iter()
        .find(|(column, _)| *column == id)
        .map(|(_, width)| *width)
        .unwrap_or_default()
}

#[test]
fn spare_space_goes_to_the_name_column() {
    // 1008 - 8 = 1000 across 547 of content: the name column takes the rest.
    let (widths, overflow) = allocate_fit_to_panel(1008.0, &list()).unwrap();
    assert!(!overflow);
    assert_eq!(width_of(&widths, ColumnId::Name), 753.0);
    assert_eq!(width_of(&widths, ColumnId::Date), 112.0);
    assert_eq!(width_of(&widths, ColumnId::Type), 64.0);
    assert_eq!(width_of(&widths, ColumnId::Size), 71.0);
}

#[test]
fn the_name_column_absorbs_small_deficits_alone() {
    // Budget 515 over 547 of content: only the name column shrinks.
    let (widths, overflow) = allocate_fit_to_panel(523.0, &list()).unwrap();
    assert!(!overflow);
    assert_eq!(width_of(&widths, ColumnId::Name), 268.0);
    assert_eq!(width_of(&widths, ColumnId::Date), 112.0);
    assert_eq!(width_of(&widths, ColumnId::Type), 64.0);
    assert_eq!(width_of(&widths, ColumnId::Size), 71.0);
}

#[test]
fn other_columns_keep_their_content_until_the_name_hits_its_minimum() {
    let measured = vec![
        measurement(ColumnId::Name, 130.0, 100.0),
        measurement(ColumnId::Date, 112.0, 96.0),
        measurement(ColumnId::Type, 64.0, 38.0),
        measurement(ColumnId::Size, 71.0, 55.0),
    ];
    // Budget 340 over 377: the 30px name slack is not enough, so the name lands
    // on 100px and the others shrink by a shared factor without truncating.
    let (widths, overflow) = allocate_fit_to_panel(348.0, &measured).unwrap();
    assert!(!overflow);
    assert_eq!(width_of(&widths, ColumnId::Name), 100.0);
    // The date column stays wide enough for its complete text (content + 8).
    assert!(width_of(&widths, ColumnId::Date) >= 104.0);
    assert!(width_of(&widths, ColumnId::Type) >= 64.0);
    assert!(width_of(&widths, ColumnId::Size) >= 64.0);
    let total: f32 = widths.iter().map(|(_, width)| *width).sum();
    assert!((total - 340.0).abs() < 0.01, "total drifted: {total}");
}

#[test]
fn the_extreme_case_lands_on_the_fixed_minimums_and_reports_overflow() {
    // Budget 260 is below the 292px of fixed minimums.
    let (widths, overflow) = allocate_fit_to_panel(268.0, &list()).unwrap();
    assert!(overflow);
    assert_eq!(width_of(&widths, ColumnId::Name), 100.0);
    assert_eq!(width_of(&widths, ColumnId::Date), 64.0);
    assert_eq!(width_of(&widths, ColumnId::Type), 64.0);
    assert_eq!(width_of(&widths, ColumnId::Size), 64.0);
}

#[test]
fn a_panel_exactly_the_width_of_the_fixed_minimums_stays_fitted() {
    let (widths, overflow) = allocate_fit_to_panel(300.0, &list()).unwrap();
    assert!(!overflow);
    for id in [
        ColumnId::Name,
        ColumnId::Date,
        ColumnId::Type,
        ColumnId::Size,
    ] {
        let expected = min_width(id);
        let width = width_of(&widths, id);
        assert!(
            (width - expected).abs() < 0.01,
            "{id:?} drifted from its minimum: {width}"
        );
    }
    let total: f32 = widths.iter().map(|(_, width)| *width).sum();
    assert!(
        total <= 292.0 + 0.01,
        "fitted total above the budget: {total}"
    );
}

#[test]
fn the_fixed_minimum_floor_drops_to_64px_for_non_name_columns() {
    assert_eq!(min_width(ColumnId::Name), 100.0);
    for id in [
        ColumnId::Date,
        ColumnId::Type,
        ColumnId::Size,
        ColumnId::Tags,
        ColumnId::OriginalLocation,
    ] {
        assert_eq!(min_width(id), 64.0, "{id:?} kept the old 80px floor");
    }
}

#[test]
fn unavailable_geometry_does_not_produce_a_panel_fit() {
    let measured = vec![measurement(ColumnId::Name, 300.0, 270.0)];
    for width in [f32::NAN, f32::INFINITY, 0.0, 8.0] {
        assert!(allocate_fit_to_panel(width, &measured).is_none());
    }
    assert!(allocate_fit_to_panel(900.0, &[]).is_none());
}
