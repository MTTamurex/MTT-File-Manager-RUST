use super::*;
use crate::infrastructure::app_state_db::AppStateDb;
use std::time::Duration;

fn confirm(manager: &OrganizerManager, expected: OrganizerCommandId) -> OrganizerCommandResult {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        match manager
            .recv_event_timeout(remaining)
            .expect("command confirmation")
        {
            OrganizerEvent::CommandResult { command_id, result } if command_id == expected => {
                return result.unwrap();
            }
            _ => {}
        }
    }
}

#[test]
fn organizer_reopens_paused_and_only_resumes_previously_active_rules() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(&destination).unwrap();
    std::fs::write(source.join("file.txt"), b"text").unwrap();
    std::fs::write(source.join("file.pdf"), b"document").unwrap();
    let mut rules: Vec<_> = [(1, "txt"), (2, "pdf")]
        .into_iter()
        .map(|(id, extension)| {
            OrganizerRule::new(
                id,
                source.clone(),
                destination.clone(),
                vec![extension.into()],
                true,
            )
            .unwrap()
        })
        .collect();
    rules[1].enabled = false;
    let db_path = root.path().join("state");
    {
        let db = AppStateDb::new(db_path.clone()).unwrap();
        db.set_organizer_globally_paused(true).unwrap();
    }
    let db = Arc::new(AppStateDb::new(db_path).unwrap());
    let (operation_sender, operation_receiver) = crossbeam_channel::unbounded();
    let manager = OrganizerManager::start(
        operation_sender,
        Arc::clone(&db),
        rules.clone(),
        eframe::egui::Context::default(),
    );
    confirm(&manager, manager.set_rules(rules).unwrap());
    // Longer than the stability delay: a startup scan must not dispatch anything.
    assert!(matches!(
        operation_receiver.recv_timeout(Duration::from_millis(2500)),
        Err(crossbeam_channel::RecvTimeoutError::Timeout)
    ));
    assert_eq!(
        confirm(&manager, manager.set_global_pause(false).unwrap()),
        OrganizerCommandResult::GlobalPauseSet { paused: false }
    );
    let request = operation_receiver
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    assert!(matches!(
        request,
        FileOperationRequest::OrganizerMove { rule_id: 1, .. }
    ));
    assert!(!db.organizer_globally_paused().unwrap());
    assert!(operation_receiver.try_recv().is_err());
}
