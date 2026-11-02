use super::*;
use crate::infrastructure::organizer::{OrganizerCommandResult, OrganizerEvent};

#[test]
fn notification_batch_emits_one_summary_after_the_idle_delay() {
    let now = Instant::now();
    let mut batch = OrganizerNotificationBatch::default();
    batch.record_at(now, |batch| batch.moved += 2);
    batch.record_at(now + Duration::from_millis(500), |batch| {
        batch.skipped += 1;
        batch.record_issue("conflict".to_string());
    });

    assert!(batch
        .take_if_idle(now + Duration::from_millis(1500))
        .is_none());
    let summary = batch
        .take_if_idle(now + Duration::from_millis(2000))
        .expect("summary after idle delay");
    assert_eq!(summary.moved, 2);
    assert_eq!(summary.skipped, 1);
    assert_eq!(summary.failed, 0);
    assert_eq!(summary.issue_details, vec!["conflict"]);
    assert!(batch
        .take_if_idle(now + Duration::from_millis(4000))
        .is_none());
}

#[test]
fn notification_batch_limits_issue_details() {
    let now = Instant::now();
    let mut batch = OrganizerNotificationBatch::default();
    for index in 0..(MAX_ISSUE_DETAILS + 2) {
        batch.record_at(now, |batch| {
            batch.failed += 1;
            batch.record_issue(format!("issue {index}"));
        });
    }
    let summary = batch
        .take_if_idle(now + NOTIFICATION_IDLE_DELAY)
        .expect("summary after idle delay");
    assert_eq!(summary.issue_details.len(), MAX_ISSUE_DETAILS);
    assert_eq!(summary.additional_issues, 2);
}

#[cfg(feature = "notify-watcher")]
#[test]
fn replacement_rules_are_applied_only_after_typed_success() {
    let root = tempfile::tempdir().expect("create test directory");
    let source = root.path().join("source");
    let first_destination = root.path().join("first-destination");
    let second_destination = root.path().join("second-destination");
    std::fs::create_dir(&source).expect("create source directory");
    std::fs::create_dir(&first_destination).expect("create first destination");
    std::fs::create_dir(&second_destination).expect("create second destination");
    let initial = OrganizerRule::new(
        1,
        source.clone(),
        first_destination,
        vec!["txt".to_string()],
        false,
    )
    .expect("create initial rule");
    let replacement = OrganizerRule::new(
        1,
        source,
        second_destination,
        vec!["txt".to_string()],
        false,
    )
    .expect("create replacement rule");
    let (file_operation_sender, _file_operation_receiver) = crossbeam_channel::unbounded();
    let mut state = OrganizerState::new(
        file_operation_sender,
        std::sync::Arc::new(
            crate::infrastructure::app_state_db::AppStateDb::new_in_memory().expect("database"),
        ),
        vec![initial.clone()],
        eframe::egui::Context::default(),
    );
    let command_id = state
        .replace_rules(vec![replacement.clone()])
        .expect("queue replacement");
    assert_eq!(state.rules, vec![initial]);
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match state.manager.try_recv_event() {
            Ok(OrganizerEvent::CommandResult {
                command_id: received_id,
                result: Ok(result @ OrganizerCommandResult::RulesUpdated { .. }),
            }) if received_id == command_id => {
                state.command_succeeded(received_id, &result);
                break;
            }
            Ok(_) | Err(std::sync::mpsc::TryRecvError::Empty) if Instant::now() < deadline => {
                std::thread::yield_now();
            }
            _ => panic!("missing rules confirmation"),
        }
    }
    assert_eq!(state.rules, vec![replacement]);
}

#[cfg(feature = "notify-watcher")]
#[test]
fn rejected_rules_never_replace_visible_state() {
    let root = tempfile::tempdir().expect("create test directory");
    let first = root.path().join("first");
    let second = root.path().join("second");
    std::fs::create_dir(&first).expect("create first directory");
    std::fs::create_dir(&second).expect("create second directory");
    let initial = OrganizerRule::new(
        1,
        first.clone(),
        second.clone(),
        vec!["pdf".to_string()],
        false,
    )
    .expect("create initial rule");
    let forward = OrganizerRule::new(
        2,
        first.clone(),
        second.clone(),
        vec!["txt".to_string()],
        true,
    )
    .expect("create forward rule");
    let reverse = OrganizerRule::new(3, second, first, vec!["txt".to_string()], true)
        .expect("create reverse rule");
    let (file_operation_sender, _file_operation_receiver) = crossbeam_channel::unbounded();
    let mut state = OrganizerState::new(
        file_operation_sender,
        std::sync::Arc::new(
            crate::infrastructure::app_state_db::AppStateDb::new_in_memory().expect("database"),
        ),
        vec![initial.clone()],
        eframe::egui::Context::default(),
    );
    let command_id = state
        .replace_rules(vec![forward, reverse])
        .expect("queue invalid replacement");
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match state.manager.try_recv_event() {
            Ok(OrganizerEvent::CommandResult {
                command_id: received_id,
                result: Err(_),
            }) if received_id == command_id => {
                state.command_failed(received_id);
                break;
            }
            Ok(_) | Err(std::sync::mpsc::TryRecvError::Empty) if Instant::now() < deadline => {
                std::thread::yield_now();
            }
            _ => panic!("missing rules rejection"),
        }
    }
    assert_eq!(state.rules, vec![initial]);
}
