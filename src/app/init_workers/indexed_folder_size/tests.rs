use super::*;
use crate::infrastructure::drive_indexing::DriveIndexingSnapshot;
use mtt_search_protocol::DriveIndexingEntry;

fn config(enabled: bool) -> DriveIndexingSnapshot {
    DriveIndexingSnapshot::new(
        vec![DriveIndexingEntry {
            drive_letter: 'R',
            enabled,
        }],
        true,
        vec![],
    )
    .unwrap()
}

#[test]
fn disabled_cached_drive_fails_without_any_ipc_or_retry() {
    let policy = SharedDriveIndexing::from_cache(None);
    policy.confirm(config(false));
    let result = query_with(
        Path::new(r"R:\folder"),
        &AtomicBool::new(false),
        &policy,
        |_| panic!("no IPC"),
    );
    assert_eq!(result.unwrap_err(), DRIVE_INDEXING_DISABLED_ERROR);
    assert!(!is_retryable_folder_size_service_error(
        DRIVE_INDEXING_DISABLED_ERROR
    ));
}

#[test]
fn changing_policy_during_an_ipc_call_discards_its_result() {
    let policy = SharedDriveIndexing::from_cache(None);
    let result = query_with(
        Path::new(r"R:\folder"),
        &AtomicBool::new(false),
        &policy,
        |_| {
            policy.confirm(config(false));
            Ok((100, 1, 1))
        },
    );
    assert_eq!(result.unwrap_err(), DRIVE_INDEXING_DISABLED_ERROR);
}

#[test]
fn disable_during_a_retry_does_not_wait_for_the_retry_deadline() {
    let policy = SharedDriveIndexing::from_cache(None);
    let mut calls = 0;
    let result = query_with(
        Path::new(r"R:\folder"),
        &AtomicBool::new(false),
        &policy,
        |_| {
            calls += 1;
            policy.confirm(config(false));
            Err("Sizes not loaded".into())
        },
    );
    assert_eq!(result.unwrap_err(), DRIVE_INDEXING_DISABLED_ERROR);
    assert_eq!(calls, 1);
}

#[test]
fn legacy_service_uses_the_existing_indexed_query_instead_of_an_obsolete_cache() {
    let policy = SharedDriveIndexing::from_cache(None);
    policy.confirm(config(false));
    policy.unavailable(
        &crate::infrastructure::drive_indexing::DriveIndexingError::new(
            crate::infrastructure::drive_indexing::DriveIndexingErrorKind::Unsupported,
            "legacy",
        ),
    );
    assert_eq!(
        query_with(
            Path::new(r"R:\folder"),
            &AtomicBool::new(false),
            &policy,
            |_| Ok((100, 1, 1))
        )
        .unwrap(),
        (100, 1, 1)
    );
}
