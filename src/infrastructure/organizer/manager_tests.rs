use super::{
    allocate_command_id, OrganizerCommand, OrganizerCommandError, OrganizerCommandId,
    OrganizerCommandResult, OrganizerEvent, OrganizerManager, PendingCommandRegistry,
};
use crate::domain::organizer_rule::{OrganizerRule, OrganizerRuleError};
use std::sync::{atomic::AtomicU64, Arc, Barrier};
use std::time::Duration;

#[test]
fn allocated_command_ids_are_non_zero_and_unique() {
    let first = OrganizerCommandId::allocate().expect("allocate first id");
    let second = OrganizerCommandId::allocate().expect("allocate second id");

    assert_ne!(first.get(), 0);
    assert_ne!(first, second);
}

#[test]
fn command_id_allocator_includes_the_maximum_value_before_exhaustion() {
    let counter = AtomicU64::new(u64::MAX);
    let maximum = allocate_command_id(&counter).expect("allocate maximum id");

    assert_eq!(maximum.get(), u64::MAX);
    assert_eq!(counter.load(std::sync::atomic::Ordering::Relaxed), 0);
    assert!(allocate_command_id(&counter).is_none());
}

#[test]
fn runtime_stop_and_command_registration_are_atomic() {
    for _ in 0..32 {
        let registry = PendingCommandRegistry::running();
        let worker_registry = registry.clone();
        let command_id = OrganizerCommandId::allocate().expect("allocate command id");
        let (command_sender, command_receiver) = std::sync::mpsc::channel();
        let barrier = Arc::new(Barrier::new(2));
        let worker_barrier = barrier.clone();
        let enqueue = std::thread::spawn(move || {
            worker_barrier.wait();
            worker_registry.register_and_send(
                command_id,
                &command_sender,
                OrganizerCommand::Refresh { command_id },
            )
        });

        barrier.wait();
        let stopped_ids = registry.stop();
        let enqueue_result = enqueue.join().expect("join enqueue thread");

        if enqueue_result.is_ok() {
            assert_eq!(stopped_ids, vec![command_id]);
            assert!(command_receiver.try_recv().is_ok());
        } else {
            assert!(stopped_ids.is_empty());
            assert!(command_receiver.try_recv().is_err());
        }
    }
}

#[cfg(feature = "notify-watcher")]
fn receive_command_result(
    manager: &OrganizerManager,
    expected_id: OrganizerCommandId,
) -> Result<OrganizerCommandResult, OrganizerCommandError> {
    loop {
        match manager
            .recv_event_timeout(Duration::from_secs(2))
            .expect("organizer command result")
        {
            OrganizerEvent::CommandResult { command_id, result } if command_id == expected_id => {
                return result
            }
            _ => {}
        }
    }
}

#[cfg(feature = "notify-watcher")]
fn start_manager(rules: Vec<OrganizerRule>) -> OrganizerManager {
    let (file_operation_sender, _file_operation_receiver) = crossbeam_channel::unbounded();
    OrganizerManager::start(
        file_operation_sender,
        Arc::new(
            crate::infrastructure::app_state_db::AppStateDb::new_in_memory().expect("database"),
        ),
        rules,
        eframe::egui::Context::default(),
    )
}

#[cfg(feature = "notify-watcher")]
#[test]
fn commands_return_typed_success_confirmations() {
    let root = tempfile::tempdir().expect("create test directory");
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    std::fs::create_dir(&source).expect("create source directory");
    std::fs::create_dir(&destination).expect("create destination directory");
    let rule = OrganizerRule::new(
        7,
        source.clone(),
        destination.clone(),
        vec!["txt".to_string()],
        true,
    )
    .expect("create rule");
    let manager = start_manager(vec![rule.clone()]);

    let rules_id = manager.set_rules(vec![rule]).expect("queue rules");
    assert_eq!(
        receive_command_result(&manager, rules_id),
        Ok(OrganizerCommandResult::RulesUpdated { rule_count: 1 })
    );
    let pause_id = manager.pause_rule(7).expect("queue pause");
    assert_eq!(
        receive_command_result(&manager, pause_id),
        Ok(OrganizerCommandResult::RulePaused { rule_id: 7 })
    );
    let resume_id = manager.resume_rule(7).expect("queue resume");
    assert_eq!(
        receive_command_result(&manager, resume_id),
        Ok(OrganizerCommandResult::RuleResumed { rule_id: 7 })
    );
    let run_id = manager.run_rule_now(7).expect("queue run");
    assert_eq!(
        receive_command_result(&manager, run_id),
        Ok(OrganizerCommandResult::RuleRunQueued { rule_id: 7 })
    );
    let refresh_id = manager.refresh().expect("queue refresh");
    assert_eq!(
        receive_command_result(&manager, refresh_id),
        Ok(OrganizerCommandResult::RefreshQueued {
            enabled_rule_count: 1
        })
    );
    let folder_id = manager
        .create_missing_folder(7, true)
        .expect("queue folder creation");
    assert_eq!(
        receive_command_result(&manager, folder_id),
        Ok(OrganizerCommandResult::FolderReady {
            rule_id: 7,
            source: true,
            path: source,
        })
    );
}

#[cfg(feature = "notify-watcher")]
#[test]
fn rejected_command_returns_typed_error_without_stopping_runtime() {
    let manager = start_manager(Vec::new());
    let root = tempfile::tempdir().expect("create test directory");
    let first_folder = root.path().join("first");
    let second_folder = root.path().join("second");
    std::fs::create_dir(&first_folder).expect("create first directory");
    std::fs::create_dir(&second_folder).expect("create second directory");
    let first_rule = OrganizerRule {
        id: 1,
        source_folder: first_folder.clone(),
        destination_folder: second_folder.clone(),
        extensions: vec!["txt".to_string()],
        enabled: true,
    };
    let second_rule = OrganizerRule {
        id: 2,
        source_folder: second_folder,
        destination_folder: first_folder,
        extensions: vec!["txt".to_string()],
        enabled: true,
    };

    let invalid_id = manager
        .set_rules(vec![first_rule, second_rule])
        .expect("queue rules");
    assert_eq!(
        receive_command_result(&manager, invalid_id),
        Err(OrganizerCommandError::InvalidRules(
            OrganizerRuleError::RuleCycle
        ))
    );

    let refresh_id = manager.refresh().expect("queue refresh after rejection");
    assert_eq!(
        receive_command_result(&manager, refresh_id),
        Ok(OrganizerCommandResult::RefreshQueued {
            enabled_rule_count: 0
        })
    );
}

#[cfg(feature = "notify-watcher")]
#[test]
fn duplicate_rule_ids_are_rejected_as_ambiguous() {
    let manager = start_manager(Vec::new());
    let root = tempfile::tempdir().expect("create test directory");
    let source = root.path().join("source");
    let first_destination = root.path().join("first-destination");
    let second_destination = root.path().join("second-destination");
    std::fs::create_dir(&source).expect("create source directory");
    std::fs::create_dir(&first_destination).expect("create first destination");
    std::fs::create_dir(&second_destination).expect("create second destination");
    let first_rule = OrganizerRule::new(
        9,
        source.clone(),
        first_destination,
        vec!["txt".to_string()],
        false,
    )
    .expect("create first rule");
    let second_rule = OrganizerRule::new(
        9,
        source,
        second_destination,
        vec!["pdf".to_string()],
        false,
    )
    .expect("create second rule");

    let command_id = manager
        .set_rules(vec![first_rule, second_rule])
        .expect("queue duplicate rules");

    assert_eq!(
        receive_command_result(&manager, command_id),
        Err(OrganizerCommandError::DuplicateRuleId { rule_id: 9 })
    );
}

#[cfg(not(feature = "notify-watcher"))]
#[test]
fn commands_fail_immediately_when_watcher_is_disabled() {
    let (file_operation_sender, _file_operation_receiver) = crossbeam_channel::unbounded();
    let manager = OrganizerManager::start(
        file_operation_sender,
        Arc::new(
            crate::infrastructure::app_state_db::AppStateDb::new_in_memory().expect("database"),
        ),
        Vec::new(),
        eframe::egui::Context::default(),
    );

    assert_eq!(
        manager.refresh(),
        Err(OrganizerCommandError::ManagerUnavailable)
    );
}
