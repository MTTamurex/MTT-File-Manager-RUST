use super::*;
use crate::domain::file_entry::FileEntry;

struct Fixture {
    state: ColumnState,
    results: Sender<(ValueKey, Option<PropertyDisplayValue>)>,
    requests: Receiver<display_worker::DisplayRequest>,
    catalog: Sender<Result<Vec<PropertyColumn>, ()>>,
}

impl Fixture {
    fn new() -> Self {
        let (sender, requests) = bounded(2);
        let (results, receiver) = bounded(2);
        let (catalog, catalog_receiver) = bounded(1);
        Self {
            state: ColumnState {
                content_revision: 0,
                preferences: ColumnPreferences::default(),
                catalog: Vec::new(),
                catalog_loaded: false,
                catalog_failed: false,
                show_picker: false,
                picker_owner: None,
                picker_search: String::new(),
                sender: Some(sender),
                receiver,
                catalog_receiver,
                values: LruCache::new(NonZeroUsize::new(2).unwrap()),
                pending: HashMap::new(),
                display_sources: Default::default(),
                sorting: sorting::ColumnSorting::new(),
            },
            results,
            requests,
            catalog,
        }
    }
}

fn key(item: &FileEntry, generation: usize) -> ValueKey {
    ValueKey {
        path: item.path.clone(),
        modified: item.modified,
        size: item.size,
        created: item.created,
        is_dir: item.is_dir,
        generation,
        property: PropertyId { fmtid: 0, pid: 1 },
    }
}

#[test]
fn deferred_values_retry_after_the_backoff_instead_of_staying_empty_forever() {
    let mut fixture = Fixture::new();
    let item = FileEntry::from_path("file.txt".into(), false);
    let key = key(&item, 1);
    fixture.results.send((key.clone(), None)).unwrap();
    fixture.state.poll();
    let ctx = egui::Context::default();
    assert_eq!(fixture.state.value(&ctx, &item, 1, key.property), "");
    assert!(fixture.requests.is_empty());
    fixture.state.values.get_mut(&key).unwrap().retry_at =
        Some(Instant::now() - Duration::from_secs(1));
    fixture.state.value(&ctx, &item, 1, key.property);
    assert_eq!(fixture.requests.try_recv().unwrap().keys, vec![key.clone()]);
    fixture
        .results
        .send((
            key.clone(),
            Some(PropertyDisplayValue::Text("value".into())),
        ))
        .unwrap();
    fixture.state.poll();
    assert_eq!(fixture.state.value(&ctx, &item, 1, key.property), "value");
    assert!(!fixture.state.pending.contains_key(&key));
}

#[test]
fn absent_properties_do_not_create_a_retry_loop() {
    let mut fixture = Fixture::new();
    let item = FileEntry::from_path("file.txt".into(), false);
    let key = key(&item, 1);
    fixture
        .results
        .send((key.clone(), Some(PropertyDisplayValue::Text(String::new()))))
        .unwrap();
    fixture.state.poll();
    assert!(fixture.state.values.get(&key).unwrap().retry_at.is_none());
    fixture
        .state
        .value(&egui::Context::default(), &item, 1, key.property);
    assert!(fixture.requests.is_empty());
}

#[test]
fn a_disconnected_catalog_does_not_leave_the_picker_loading_forever() {
    let mut fixture = Fixture::new();
    drop(fixture.catalog);
    fixture.state.poll();
    assert!(fixture.state.catalog_loaded);
    assert!(fixture.state.catalog_failed);
}

#[test]
fn a_disconnected_display_worker_releases_pending_requests() {
    let mut fixture = Fixture::new();
    fixture.state.pending.insert(
        key(&FileEntry::from_path("file.txt".into(), false), 1),
        Default::default(),
    );
    drop(fixture.results);
    fixture.state.poll();
    assert!(fixture.state.sender.is_none());
    assert!(fixture.state.pending.is_empty());
}

#[test]
fn a_previous_generation_cannot_supply_the_current_cell_value() {
    let mut fixture = Fixture::new();
    let item = FileEntry::from_path("file.txt".into(), false);
    let old = key(&item, 1);
    fixture
        .results
        .send((
            old.clone(),
            Some(PropertyDisplayValue::Text("old value".into())),
        ))
        .unwrap();
    fixture.state.poll();
    assert_eq!(
        fixture
            .state
            .value(&egui::Context::default(), &item, 2, old.property),
        ""
    );
    assert_eq!(fixture.requests.try_recv().unwrap().keys[0].generation, 2);
}

#[test]
fn replacing_a_file_invalidates_values_even_with_the_same_size_and_modified_time() {
    let mut fixture = Fixture::new();
    let mut item = FileEntry::from_path("file.txt".into(), false);
    item.created = Some(1);
    let old = key(&item, 1);
    fixture
        .results
        .send((
            old.clone(),
            Some(PropertyDisplayValue::Text("old value".into())),
        ))
        .unwrap();
    fixture.state.poll();
    item.created = Some(2);
    assert_eq!(
        fixture
            .state
            .value(&egui::Context::default(), &item, 1, old.property),
        ""
    );
    assert_eq!(
        fixture.requests.try_recv().unwrap().keys[0].created,
        Some(2)
    );
}

#[test]
fn cached_dates_can_follow_each_date_preference_without_reading_the_file_again() {
    use crate::domain::date_format::DateFormat;
    let mut fixture = Fixture::new();
    let item = FileEntry::from_path("file.txt".into(), false);
    let key = key(&item, 1);
    let timestamp = 1_770_638_400;
    let ticks = (timestamp + 11_644_473_600) * 10_000_000;
    fixture
        .results
        .send((key.clone(), Some(PropertyDisplayValue::Date(ticks))))
        .unwrap();
    fixture.state.poll();
    for format in [
        DateFormat::DayMonthYear,
        DateFormat::MonthDayYear,
        DateFormat::YearMonthDay,
    ] {
        let value = &fixture.state.values.get(&key).unwrap().value;
        assert_eq!(
            value.text_with_format(format),
            crate::infrastructure::windows::formatting::format_date_with_format(timestamp, format)
        );
    }
    fixture
        .state
        .value(&egui::Context::default(), &item, 1, key.property);
    assert!(fixture.requests.is_empty());
}

#[test]
fn known_creation_dates_are_available_on_first_frame_without_worker_requests() {
    let created = PropertyId {
        fmtid: 0xb725f130_47ef_101a_a5f1_02608c9eebac,
        pid: 15,
    };
    for is_dir in [false, true] {
        let mut fixture = Fixture::new();
        // No real entry is needed: the listing already contains all required metadata.
        let mut item = FileEntry::from_path("entry.txt".into(), is_dir);
        item.created = Some(1_770_638_400);
        assert_eq!(
            fixture
                .state
                .value(&egui::Context::default(), &item, 1, created),
            crate::infrastructure::windows::format_date(item.created.unwrap())
        );
        assert!(fixture.requests.is_empty());
        assert!(fixture.state.pending.is_empty());
        assert!(fixture.state.values.is_empty());
    }
}

#[test]
fn missing_creation_dates_still_use_the_async_fallback() {
    let created = PropertyId {
        fmtid: 0xb725f130_47ef_101a_a5f1_02608c9eebac,
        pid: 15,
    };
    for timestamp in [None, Some(0)] {
        let mut fixture = Fixture::new();
        let mut item = FileEntry::from_path("file.txt".into(), false);
        item.created = timestamp;
        assert_eq!(
            fixture
                .state
                .value(&egui::Context::default(), &item, 1, created),
            ""
        );
        assert_eq!(
            fixture.requests.try_recv().unwrap().keys[0].property,
            created
        );
    }
}

#[test]
fn creation_metadata_does_not_bypass_other_property_requests() {
    let mut fixture = Fixture::new();
    let mut item = FileEntry::from_path("file.txt".into(), false);
    item.created = Some(1_770_638_400);
    let other = key(&item, 1).property;
    assert_eq!(
        fixture
            .state
            .value(&egui::Context::default(), &item, 1, other),
        ""
    );
    assert_eq!(fixture.requests.try_recv().unwrap().keys[0].property, other);
}

#[test]
fn measuring_uncached_properties_does_not_schedule_worker_requests() {
    let fixture = Fixture::new();
    let item = FileEntry::from_path("file.txt".into(), false);
    assert!(fixture
        .state
        .cached_value(&item, 1, key(&item, 1).property)
        .is_none());
    assert!(fixture.requests.is_empty());
    assert!(fixture.state.pending.is_empty());
}

#[test]
fn metadata_arrivals_invalidate_content_measurements_without_a_new_file_listing() {
    let mut fixture = Fixture::new();
    let item = FileEntry::from_path("file.txt".into(), false);
    let key = key(&item, 1);
    let before = fixture.state.content_revision;
    fixture
        .results
        .send((
            key.clone(),
            Some(PropertyDisplayValue::Text("Example description".into())),
        ))
        .unwrap();
    fixture.state.poll();
    assert!(fixture.state.content_revision > before);
    assert_eq!(
        fixture
            .state
            .cached_value(&item, 1, key.property)
            .as_deref(),
        Some("Example description")
    );
    assert!(fixture.requests.is_empty());
}

#[path = "request_tests.rs"]
mod requests;
