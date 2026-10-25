use super::*;

#[test]
fn enabled_properties_are_queued_once_in_a_single_file_request() {
    let mut fixture = Fixture::new();
    let item = FileEntry::from_path("file.txt".into(), false);
    let properties = [
        PropertyId { fmtid: 1, pid: 1 },
        PropertyId { fmtid: 2, pid: 2 },
    ];
    let ctx = egui::Context::default();
    fixture.state.request_values(
        &ctx,
        &item,
        1,
        [properties[0], properties[1], properties[0]],
    );
    let request = fixture.requests.try_recv().unwrap();
    assert_eq!(
        request
            .keys
            .iter()
            .map(|key| key.property)
            .collect::<Vec<_>>(),
        properties
    );
    assert_eq!(fixture.state.pending.len(), 2);
    for property in properties {
        assert_eq!(fixture.state.value(&ctx, &item, 1, property), "");
    }
    assert!(fixture.requests.is_empty());
}

#[test]
fn batching_skips_known_creation_dates_and_already_cached_properties() {
    let mut fixture = Fixture::new();
    let mut item = FileEntry::from_path("file.txt".into(), false);
    item.created = Some(1_770_638_400);
    let cached = key(&item, 1);
    fixture.state.values.put(
        cached.clone(),
        CachedValue::new(
            Some(PropertyDisplayValue::Text("value".into())),
            Instant::now(),
        ),
    );
    let missing = PropertyId { fmtid: 2, pid: 3 };
    fixture.state.request_values(
        &egui::Context::default(),
        &item,
        1,
        [column_properties::DATE_CREATED, cached.property, missing],
    );
    let request = fixture.requests.try_recv().unwrap();
    assert_eq!(request.keys.len(), 1);
    assert_eq!(request.keys[0].property, missing);
    assert_eq!(fixture.state.pending.len(), 1);
}

#[test]
fn a_full_file_queue_does_not_leave_unqueued_cells_pending() {
    let mut fixture = Fixture::new();
    let ctx = egui::Context::default();
    let property = PropertyId { fmtid: 1, pid: 1 };
    for name in ["first.txt", "second.txt"] {
        let item = FileEntry::from_path(name.into(), false);
        fixture.state.request_values(&ctx, &item, 1, [property]);
    }
    let item = FileEntry::from_path("third.txt".into(), false);
    fixture.state.request_values(&ctx, &item, 1, [property]);
    assert!(!fixture
        .state
        .pending
        .contains_key(&ValueKey::new(&item, 1, property)));
    fixture.requests.try_recv().unwrap();
    fixture.state.request_values(&ctx, &item, 1, [property]);
    assert!(fixture
        .state
        .pending
        .contains_key(&ValueKey::new(&item, 1, property)));
}

#[test]
fn file_batches_preserve_deferred_backoff_without_delaying_other_properties() {
    let mut fixture = Fixture::new();
    let item = FileEntry::from_path("file.txt".into(), false);
    let deferred = key(&item, 1);
    fixture
        .state
        .values
        .put(deferred.clone(), CachedValue::new(None, Instant::now()));
    let other = PropertyId { fmtid: 2, pid: 2 };
    fixture.state.request_values(
        &egui::Context::default(),
        &item,
        1,
        [deferred.property, other],
    );
    let request = fixture.requests.try_recv().unwrap();
    assert_eq!(request.keys.len(), 1);
    assert_eq!(request.keys[0].property, other);
}

#[test]
fn navigation_cancels_only_the_changed_pane_and_releases_its_pending_cells() {
    let mut fixture = Fixture::new();
    let ctx = egui::Context::default();
    let first = FileEntry::from_path(r"C:\Example\first.txt".into(), false);
    let second = FileEntry::from_path(r"D:\Example\second.txt".into(), false);
    let property = key(&first, 1).property;
    fixture
        .state
        .set_display_source(0, Some((1, std::path::Path::new(r"C:\Example"))));
    fixture.state.request_values(&ctx, &first, 1, [property]);
    let left = fixture.requests.try_recv().unwrap();
    fixture
        .state
        .set_display_source(1, Some((1, std::path::Path::new(r"D:\Example"))));
    fixture.state.request_values(&ctx, &second, 1, [property]);
    let right = fixture.requests.try_recv().unwrap();
    fixture
        .state
        .set_display_source(0, Some((2, std::path::Path::new(r"C:\Other"))));
    assert!(left.cancelled.load(Ordering::Acquire));
    assert!(!right.cancelled.load(Ordering::Acquire));
    assert_eq!(fixture.state.pending.len(), 1);
    assert!(fixture
        .state
        .pending
        .contains_key(&ValueKey::new(&second, 1, property)));
}

#[test]
fn unchanged_navigation_preserves_in_flight_requests_and_deduplication() {
    let mut fixture = Fixture::new();
    let ctx = egui::Context::default();
    let item = FileEntry::from_path("file.txt".into(), false);
    let property = key(&item, 1).property;
    for _ in 0..2 {
        fixture
            .state
            .set_display_source(0, Some((1, std::path::Path::new(r"C:\Example"))));
        fixture.state.request_values(&ctx, &item, 1, [property]);
    }
    let request = fixture.requests.try_recv().unwrap();
    assert!(!request.cancelled.load(Ordering::Acquire));
    assert!(fixture.requests.is_empty());
}
