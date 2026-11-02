use super::super::supervisor::WatcherRuntime;
use super::*;
use crate::domain::organizer_rule::OrganizerConflictPolicy;

fn with_context(test: impl FnOnce(&mut CommandContext<'_>, &Receiver<OrganizerEvent>)) {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(&destination).unwrap();
    std::fs::write(source.join("file.txt"), b"content").unwrap();
    let mut rules = vec![OrganizerRule {
        id: 1,
        source_folder: source,
        destination_folder: destination,
        extensions: vec!["txt".to_string()],
        enabled: true,
        conflict_policy: OrganizerConflictPolicy::Ask,
    }];
    let db = Arc::new(AppStateDb::new_in_memory().unwrap());
    let mut activations = super::super::queue::activation_flags_for(&rules);
    let mut paused = HashSet::new();
    let mut global = false;
    let mut pending = HashMap::new();
    let (watch_sender, _watch_receiver) = mpsc::sync_channel(16);
    let mut watcher = WatcherRuntime::new(watch_sender, Arc::new(AtomicBool::new(false)));
    let mut statuses = HashMap::new();
    let (event_sender, event_receiver) = mpsc::channel();
    let (operation_sender, _operation_receiver) = crossbeam_channel::unbounded();
    let mut context = CommandContext {
        rules: &mut rules,
        activation_flags: &mut activations,
        paused_rules: &mut paused,
        globally_paused: &mut global,
        pending: &mut pending,
        watcher_runtime: &mut watcher,
        reported_statuses: &mut statuses,
        event_sender: &event_sender,
        ui_ctx: &eframe::egui::Context::default(),
        pending_commands: &PendingCommandRegistry::running(),
        app_state_db: &db,
        in_flight: &OrganizerInFlightRegistry::default(),
        undo_exemptions: &OrganizerUndoExemptionRegistry::default(),
        file_operation_sender: &operation_sender,
        shutdown: &Arc::new(AtomicBool::new(false)),
    };
    test(&mut context, &event_receiver);
}

fn command_id() -> OrganizerCommandId {
    OrganizerCommandId::allocate().unwrap()
}

#[test]
fn global_switch_preserves_running_tokens_and_individual_toggle_positions() {
    with_context(|context, _| {
        let running_token = context.activation_flags[&1].clone();
        let original_rules = context.rules.clone();
        context.set_global_pause(command_id(), true);
        assert_eq!(*context.rules, original_rules);
        assert!(running_token.load(Ordering::Acquire));
        assert!(Arc::ptr_eq(&running_token, &context.activation_flags[&1]));
        context.set_global_pause(command_id(), false);
        assert_eq!(*context.rules, original_rules);
        assert!(context
            .pending
            .contains_key(&context.rules[0].source_folder.join("file.txt")));
    });
}

#[test]
fn rules_added_or_edited_during_global_pause_cannot_queue_work() {
    with_context(|context, _| {
        context.set_global_pause(command_id(), true);
        let mut rules = context.rules.clone();
        rules[0].conflict_policy = OrganizerConflictPolicy::Skip;
        let mut additional = rules[0].clone();
        additional.id = 2;
        additional.extensions = vec!["pdf".to_string()];
        std::fs::write(additional.source_folder.join("file.pdf"), b"content").unwrap();
        rules.push(additional);
        context.process(OrganizerCommand::SetRules {
            command_id: command_id(),
            rules,
        });
        assert_eq!(*context.paused_rules, HashSet::from([1, 2]));
        assert!(context.pending.is_empty());
        context.set_global_pause(command_id(), false);
        assert_eq!(context.pending.len(), 2);
    });
}

#[test]
fn updating_individual_rules_cannot_bypass_global_switch() {
    with_context(|context, _| {
        context.set_global_pause(command_id(), true);
        for enabled in [false, true] {
            let mut rules = context.rules.clone();
            rules[0].enabled = enabled;
            context.process(OrganizerCommand::SetRules {
                command_id: command_id(),
                rules,
            });
            assert!(context.pending.is_empty());
            assert!(*context.globally_paused);
        }
    });
}

#[test]
fn pause_write_failure_leaves_runtime_state_unchanged() {
    with_context(|context, events| {
        context
            .app_state_db
            .test_hold_writer()
            .execute_batch(
                "CREATE TRIGGER reject_pause BEFORE INSERT ON user_preferences
             BEGIN SELECT RAISE(FAIL, 'test failure'); END;",
            )
            .unwrap();
        context.set_global_pause(command_id(), true);
        assert!(!*context.globally_paused);
        assert!(context.paused_rules.is_empty());
        assert!(!context.app_state_db.organizer_globally_paused().unwrap());
        assert!(matches!(
            events.try_recv().unwrap(),
            OrganizerEvent::CommandResult {
                result: Err(OrganizerCommandError::PausePersistenceFailed { .. }),
                ..
            }
        ));
        assert!(context.activation_flags[&1].load(Ordering::Acquire));
    });
}

#[test]
fn startup_restores_global_switch_and_fails_closed() {
    let db = AppStateDb::new_in_memory().unwrap();
    let (sender, receiver) = mpsc::channel();
    db.set_organizer_globally_paused(true).unwrap();
    let global = load_pause_state(&db, &sender);
    assert!(global);
    assert!(receiver.try_recv().is_err());
    db.set_preference("organizer_globally_paused", "invalid")
        .unwrap();
    assert!(load_pause_state(&db, &sender));
    assert!(matches!(
        receiver.try_recv().unwrap(),
        OrganizerEvent::Error { .. }
    ));
}

#[test]
fn disabled_rules_stay_disabled_after_global_resume() {
    with_context(|context, _| {
        let mut disabled = context.rules[0].clone();
        disabled.id = 2;
        disabled.enabled = false;
        disabled.extensions = vec!["pdf".into()];
        std::fs::write(disabled.source_folder.join("file.pdf"), b"content").unwrap();
        let mut rules = context.rules.clone();
        rules.push(disabled);
        context.process(OrganizerCommand::SetRules {
            command_id: command_id(),
            rules,
        });
        context.set_global_pause(command_id(), true);
        context.set_global_pause(command_id(), false);
        assert!(!context.rules[1].enabled);
        assert!(!context.activation_flags[&2].load(Ordering::Acquire));
        assert!(context.pending.values().all(|pending| pending.rule.id == 1));
        let mut rules = context.rules.clone();
        rules[1].enabled = true;
        context.process(OrganizerCommand::SetRules {
            command_id: command_id(),
            rules,
        });
        assert!(context.rules[1].enabled);
        assert_eq!(context.pending.len(), 2);
    });
}

#[test]
fn failed_global_enable_keeps_all_rules_globally_disabled() {
    with_context(|context, events| {
        context.set_global_pause(command_id(), true);
        context
            .app_state_db
            .test_hold_writer()
            .execute_batch(
                "CREATE TRIGGER reject_global_enable BEFORE INSERT ON user_preferences
                 BEGIN SELECT RAISE(FAIL, 'test failure'); END;",
            )
            .unwrap();
        context.set_global_pause(command_id(), false);
        assert!(*context.globally_paused);
        assert!(context.app_state_db.organizer_globally_paused().unwrap());
        assert!(context.pending.is_empty());
        assert!(matches!(
            events.try_iter().last(),
            Some(OrganizerEvent::CommandResult {
                result: Err(OrganizerCommandError::PausePersistenceFailed { .. }),
                ..
            })
        ));
    });
}
