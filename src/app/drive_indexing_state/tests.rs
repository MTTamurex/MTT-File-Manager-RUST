use super::*;
use crate::infrastructure::drive_indexing::DriveIndexingSnapshot;
use mtt_search_protocol::DriveIndexingEntry;

fn policy() -> SharedDriveIndexing {
    let shared = SharedDriveIndexing::from_cache(None);
    shared.confirm(DriveIndexingSnapshot::new(vec![], true, vec![]).unwrap());
    shared
}

#[test]
fn queued_changes_are_not_optimistic_and_each_letter_has_one_pending_operation() {
    let shared = policy();
    let view = shared.snapshot();
    let mut pending = BTreeMap::new();
    let mut id = 0;
    assert!(queue_change(&view, &mut pending, &mut id, 'r', false, |_| Ok(())).unwrap());
    assert!(view.is_enabled('R'));
    assert!(shared.is_enabled('R'));
    assert_eq!(pending.len(), 1);
    assert!(
        queue_change(&view, &mut pending, &mut id, 'R', true, |_| panic!(
            "must not submit twice"
        ))
        .is_err()
    );
    assert!(queue_change(&view, &mut pending, &mut id, 'S', false, |_| Ok(())).unwrap());
}

#[test]
fn failed_queue_submission_does_not_leave_a_pending_change() {
    let view = policy().snapshot();
    let mut pending = BTreeMap::new();
    assert!(
        queue_change(&view, &mut pending, &mut 0, 'R', false, |_| Err(
            DriveIndexingError::transport("closed")
        ))
        .is_err()
    );
    assert!(pending.is_empty());
}

#[test]
fn only_confirmed_snapshots_dirty_the_persisted_cache() {
    let shared = policy();
    let mut state = DriveIndexingState::from_shared(shared.clone());
    assert!(!state.poll().cache_changed);
    shared.unavailable(&DriveIndexingError::transport("offline"));
    assert!(!state.poll().cache_changed);
    shared.confirm(
        DriveIndexingSnapshot::new(
            vec![DriveIndexingEntry {
                drive_letter: 'R',
                enabled: false,
            }],
            true,
            vec![],
        )
        .unwrap(),
    );
    let changed = state.poll();
    assert!(changed.cache_changed);
    assert!(changed.policy_changed);
    assert!(!state.poll().cache_changed);
}

#[test]
fn offline_or_legacy_service_cannot_accept_edits() {
    for kind in [
        DriveIndexingErrorKind::Transport,
        DriveIndexingErrorKind::Unsupported,
        DriveIndexingErrorKind::AccessDenied,
    ] {
        let shared = policy();
        shared.unavailable(&DriveIndexingError::new(kind, "test failure"));
        assert!(queue_change(
            &shared.snapshot(),
            &mut BTreeMap::new(),
            &mut 0,
            'R',
            false,
            |_| panic!("must not send")
        )
        .is_err());
    }
}

#[test]
fn startup_preferences_load_the_validated_cache() {
    let db = crate::infrastructure::app_state_db::AppStateDb::new_in_memory().unwrap();
    db.set_preference("language", "en").unwrap();
    db.set_preference(
        crate::infrastructure::drive_indexing::CACHE_PREFERENCE_KEY,
        r#"[{"drive_letter":"R","enabled":false}]"#,
    )
    .unwrap();
    assert!(!crate::app::init_preferences::StartupPreferences::load(&db)
        .drive_indexing
        .is_enabled('R'));
}

#[test]
fn stale_operation_ids_do_not_finish_a_newer_pending_change() {
    use crate::workers::drive_indexing_worker::{
        DriveIndexingOperation, DriveIndexingWorkerStatus,
    };
    let mut state = DriveIndexingState::from_shared(policy());
    state.pending.insert(
        'R',
        DriveIndexingChange {
            request_id: 2,
            drive_letter: 'R',
            enabled: false,
        },
    );
    state.apply_status(DriveIndexingWorkerStatus {
        operations: BTreeMap::from([(
            'R',
            DriveIndexingOperation {
                request_id: 1,
                requested_enabled: false,
                outcome: DriveIndexingWriteOutcome::Confirmed { enabled: false },
            },
        )]),
        refresh_error: None,
    });
    assert_eq!(state.pending.len(), 1);
}

#[test]
fn success_on_one_drive_does_not_hide_another_drives_failure() {
    use crate::workers::drive_indexing_worker::{
        DriveIndexingOperation, DriveIndexingWorkerStatus,
    };
    let mut state = DriveIndexingState::from_shared(policy());
    for letter in ['R', 'S'] {
        state.pending.insert(
            letter,
            DriveIndexingChange {
                request_id: 1,
                drive_letter: letter,
                enabled: false,
            },
        );
    }
    let error = DriveIndexingError::service("Drive configuration update failed".into());
    state.apply_status(DriveIndexingWorkerStatus {
        operations: BTreeMap::from([
            (
                'R',
                DriveIndexingOperation {
                    request_id: 1,
                    requested_enabled: false,
                    outcome: DriveIndexingWriteOutcome::Failed(error.clone()),
                },
            ),
            (
                'S',
                DriveIndexingOperation {
                    request_id: 1,
                    requested_enabled: false,
                    outcome: DriveIndexingWriteOutcome::Confirmed { enabled: false },
                },
            ),
        ]),
        refresh_error: None,
    });
    assert!(state.pending.is_empty());
    assert_eq!(state.errors.get(&'R'), Some(&error));
    assert!(!state.errors.contains_key(&'S'));
}

#[test]
fn rapid_drive_indexing_flips_still_invalidate_sizes_when_the_final_mask_is_unchanged() {
    let shared = policy();
    let mut state = DriveIndexingState::from_shared(shared.clone());
    shared.confirm(
        DriveIndexingSnapshot::new(
            vec![DriveIndexingEntry {
                drive_letter: 'R',
                enabled: false,
            }],
            true,
            vec![],
        )
        .unwrap(),
    );
    shared.confirm(DriveIndexingSnapshot::new(vec![], true, vec![]).unwrap());
    let changed = state.poll();
    assert!(changed.policy_changed);
    assert_ne!(changed.changed_drive_mask, 0);
}
