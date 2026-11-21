use super::*;

fn config(enabled: bool) -> DriveIndexingSnapshot {
    DriveIndexingSnapshot::new(
        vec![DriveIndexingEntry {
            drive_letter: 'R',
            enabled,
        }],
        true,
        if enabled { vec!['R'] } else { vec![] },
    )
    .unwrap()
}

#[test]
fn cached_config_is_effective_offline_but_is_not_discovery_coverage() {
    let shared = SharedDriveIndexing::from_cache(Some(r#"[{"drive_letter":"R","enabled":false}]"#));
    assert!(!shared.is_enabled('r'));
    assert!(shared.is_enabled('S'));
    assert!(shared.snapshot().coverage.is_none());
    shared.unavailable(&DriveIndexingError::transport("timeout"));
    assert!(!shared.is_enabled('R'));
    assert_eq!(shared.generation(), 0);
}

#[test]
fn invalid_cache_is_not_a_confirmed_empty_config() {
    for json in [
        "invalid",
        r#"[{"drive_letter":"r","enabled":false}]"#,
        r#"[{"drive_letter":"R","enabled":true},{"drive_letter":"R","enabled":false}]"#,
    ] {
        let shared = SharedDriveIndexing::from_cache(Some(json));
        assert!(!shared.snapshot().has_confirmed_config);
        assert!(shared.snapshot().cache_json().is_none());
        assert!(shared.is_enabled('R'));
    }
}

#[test]
fn legacy_service_does_not_lose_its_capability_when_transport_later_fails() {
    let shared = SharedDriveIndexing::from_cache(None);
    shared.confirm(config(false));
    let cache = shared.snapshot().cache_json();
    shared.unavailable(&DriveIndexingError::new(
        DriveIndexingErrorKind::Unsupported,
        "legacy",
    ));
    assert!(shared.is_enabled('R'));
    assert_eq!(shared.snapshot().cache_json(), cache);
    let generation = shared.generation();
    shared.unavailable(&DriveIndexingError::transport("timeout"));
    assert!(shared.is_enabled('R'));
    assert_eq!(shared.generation(), generation);
    shared.confirm(config(false));
    assert!(!shared.is_enabled('R'));
    assert_ne!(shared.generation(), generation);
}

#[test]
fn only_effective_policy_changes_advance_the_generation() {
    let shared = SharedDriveIndexing::from_cache(None);
    shared.confirm(config(false));
    let generation = shared.generation();
    shared.confirm(config(false));
    shared.unavailable(&DriveIndexingError::transport("offline"));
    shared.confirm(config(false));
    assert_eq!(generation, shared.generation());
    shared.confirm(config(true));
    assert_ne!(generation, shared.generation());
}

#[test]
fn snapshot_validates_sorts_and_supports_extended_drive_paths() {
    let shared = SharedDriveIndexing::from_cache(None);
    shared.confirm(
        DriveIndexingSnapshot::new(
            vec![
                DriveIndexingEntry {
                    drive_letter: 'S',
                    enabled: true,
                },
                DriveIndexingEntry {
                    drive_letter: 'R',
                    enabled: false,
                },
            ],
            true,
            vec!['S'],
        )
        .unwrap(),
    );
    assert_eq!(shared.snapshot().drives[0].drive_letter, 'R');
    for path in [r"R:\folder", r"\\?\r:\folder", "R:/folder"] {
        assert!(!shared.is_path_enabled(Path::new(path)));
    }
    assert!(shared.is_path_enabled(Path::new(r"\\server\share")));
    assert!(DriveIndexingSnapshot::new(vec![], true, vec!['R']).is_err());
}
