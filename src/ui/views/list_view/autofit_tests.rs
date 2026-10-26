use super::*;
use crate::infrastructure::windows::column_properties::DATE_CREATED;

fn natural() -> Vec<ColumnMeasurement> {
    vec![
        ColumnMeasurement {
            id: ColumnId::Name,
            width: 140.0,
        },
        ColumnMeasurement {
            id: ColumnId::Property(DATE_CREATED),
            width: 120.0,
        },
    ]
}

#[test]
fn new_columns_are_measured_from_headers_and_content_and_never_compressed() {
    let ctx = eframe::egui::Context::default();
    let _ = ctx.run_ui(Default::default(), |ui| {
        let id = ColumnId::Property(DATE_CREATED);
        let mut created = ColumnMeasurement::new(ui, id, "Date created");
        let text = "09/02/2026 12:00";
        created.include(ui, text, 16.0);
        let text_width = text_layout::get_cached_text_width(
            text,
            &FontId::proportional(12.0),
            Color32::WHITE,
            ui,
        );
        assert!(created.width >= text_width + 16.0);
        let mut tags = ColumnMeasurement::new(ui, ColumnId::Tags, "Tags");
        tags.include(ui, "Example label; Another example label", 16.0);
        let mut location =
            ColumnMeasurement::new(ui, ColumnId::OriginalLocation, "Original location");
        location.include(ui, r"C:\Users\user\Documents\Project", 16.0);
        let mut measurements = natural();
        measurements[1] = created;
        measurements.extend([tags, location]);
        let widths = allocate(220.0, &measurements).unwrap();
        assert!(widths.iter().map(|(_, width)| width).sum::<f32>() > 212.0);
        for (measured, (_, width)) in measurements.iter().zip(widths) {
            assert!(width >= measured.width);
        }
    });
}

#[test]
fn spare_space_goes_to_name_even_when_name_is_not_first() {
    let mut columns = natural();
    columns.swap(0, 1);
    let widths = allocate(900.0, &columns).unwrap();
    assert_eq!(widths[0].1, 120.0);
    assert_eq!(widths[1], (ColumnId::Name, 772.0));
}

#[test]
fn unavailable_geometry_does_not_produce_an_auto_fit() {
    for width in [f32::NAN, f32::INFINITY, 0.0, 8.0] {
        assert!(allocate(width, &natural()).is_none());
    }
    assert!(allocate(900.0, &[]).is_none());
}

#[test]
fn late_metadata_grows_widths_without_shrinking_when_rows_leave_the_viewport() {
    let mut state = FitState::default();
    state.fit(1, 1, 300.0, natural(), false);
    let mut longer = natural();
    longer[1].width = 360.0;
    state.fit(2, 1, 300.0, longer, false);
    assert_eq!(state.planned[1].1, 360.0);
    state.fit(3, 1, 300.0, natural(), false);
    assert_eq!(state.planned[1].1, 360.0);
    // A new listing may legitimately have shorter contents.
    state.fit(4, 2, 300.0, natural(), false);
    assert_eq!(state.planned[1].1, 120.0);
}

#[test]
fn unchanged_measurements_always_produce_content_widths() {
    let mut state = FitState::default();
    state.fit(1, 1, 900.0, natural(), false);
    state.planned[1].1 = 200.0;
    state.fit(2, 1, 900.0, natural(), false);
    assert_eq!(state.planned[1].1, 120.0);
    state.fit(3, 1, 900.0, natural(), true);
    assert_eq!(state.planned[1].1, 120.0);
}

#[test]
fn changing_folders_discards_manual_overrides_even_when_content_widths_are_identical() {
    let mut state = FitState::default();
    state.fit(1, 1, 900.0, natural(), false);
    let expected = state.planned.clone();
    state.planned[0].1 = 300.0;
    state.planned[1].1 = 500.0;
    state.fit(2, 2, 900.0, natural(), false);
    assert_eq!(state.planned, expected);
}

#[test]
fn reactivating_automatic_sizing_discards_previous_manual_overrides() {
    let mut state = FitState::default();
    state.fit(1, 1, 900.0, natural(), false);
    let expected = state.planned.clone();
    state.planned[1].1 = 500.0;
    state.active = false;
    state.fit(1, 1, 900.0, natural(), false);
    assert_eq!(state.planned, expected);
}

#[test]
fn empty_folder_headers_produce_automatic_widths_without_retaining_manual_sizes() {
    let context = eframe::egui::Context::default();
    let _ = context.run_ui(Default::default(), |ui| {
        let mut state = FitState::default();
        state.fit(1, 1, 900.0, natural(), false);
        state.planned[1].1 = 500.0;
        let headers = vec![
            ColumnMeasurement::new(ui, ColumnId::Name, "Name"),
            ColumnMeasurement::new(ui, ColumnId::Property(DATE_CREATED), "Date created"),
        ];
        let expected = allocate(900.0, &headers).unwrap();
        state.fit(2, 2, 900.0, headers, false);
        assert_eq!(state.planned, expected);
    });
}

#[test]
fn automatic_metadata_widths_are_independent_between_panes() {
    let mut left = FitState::default();
    let mut right = FitState::default();
    left.fit(1, 1, 400.0, natural(), false);
    let mut longer = natural();
    longer[1].width = 400.0;
    right.fit(2, 2, 400.0, longer, false);
    assert_eq!(left.planned[1].1, 120.0);
    assert_eq!(right.planned[1].1, 400.0);
}

#[test]
fn reordering_columns_keeps_content_widths_instead_of_manual_widths() {
    let mut state = FitState::default();
    state.fit(1, 1, 900.0, natural(), false);
    state.planned[1].1 = 240.0;
    let mut reordered = natural();
    reordered.swap(0, 1);
    state.fit(2, 1, 900.0, reordered, false);
    assert_eq!(
        state
            .planned
            .iter()
            .find(|(id, _)| *id == ColumnId::Property(DATE_CREATED))
            .unwrap()
            .1,
        120.0
    );
}

#[test]
fn sampling_includes_the_last_visible_row_and_discontinuous_group_indices() {
    let indices = sample_indices(2000, 5, &[201, 205, 1000, 1500]);
    for index in [201, 205, 1000, 1500] {
        assert!(indices.contains(&index));
    }
    assert_eq!(indices.len(), 204);
    assert!(!indices.contains(&500));
    assert!(sample_indices(1000, 5, &[999]).contains(&999));
}

#[test]
fn sampling_keeps_every_visible_row_without_invalid_or_duplicate_indices() {
    let visible: Vec<_> = (500..850).chain([5, 500, 1200]).collect();
    let indices = sample_indices(1000, 5, &visible);
    assert_eq!(indices.len(), 550);
    assert_eq!(indices.last(), Some(&849));
}

#[test]
fn late_folder_and_live_file_sizes_invalidate_measurements_beyond_the_baseline() {
    let indices = sample_indices(1000, 5, &[700, 701]);
    let mut sizes = vec![Some(1_u64); 1000];
    sizes[700] = None;
    let before = size_signature(indices.iter().map(|&index| sizes[index]));
    sizes[700] = Some(10_000_000_000);
    let folder_arrival = size_signature(indices.iter().map(|&index| sizes[index]));
    assert_ne!(before, folder_arrival);
    sizes[701] = Some(20_000_000_000);
    let live_update = size_signature(indices.iter().map(|&index| sizes[index]));
    assert_ne!(folder_arrival, live_update);
}

#[test]
fn first_visible_rows_and_subsequent_frames_do_not_deadlock_the_egui_context() {
    let (sender, receiver) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let context = eframe::egui::Context::default();
        for expected in [vec![0, 1, 2], vec![0, 1, 2], vec![500, 1000], vec![]] {
            let _ = context.run_ui(Default::default(), |ui| {
                let id = ui.id().with("list_content_fit");
                ui.data_mut(|data| {
                    if data.get_temp::<FitState>(id).is_none() {
                        data.insert_temp(id, FitState::default());
                    }
                });
                let mut visible = expected.clone();
                record_visible_rows(ui, &mut visible);
                let actual = ui.data_mut(|data| data.get_temp::<FitState>(id).unwrap().visible);
                assert_eq!(actual, expected);
                // Further context access must work after recording rows and requesting repaint.
                ui.ctx().request_repaint();
            });
        }
        sender.send(()).unwrap();
    });
    receiver
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("recording visible rows must complete without locking the UI");
    worker.join().unwrap();
}
