use super::*;

#[test]
fn drive_indexing_policy_change_cancels_counting_before_another_ipc_request() {
    let policy = SharedDriveIndexing::from_cache(None);
    let (sender, _) = std::sync::mpsc::channel();
    let task = CountTask {
        sender,
        ctx: eframe::egui::Context::default(),
        query: "file".into(),
        local_total_matches: 0,
        generation: 1,
        active_generation: Arc::new(AtomicU64::new(1)),
        in_flight: Arc::new(AtomicBool::new(false)),
        policy: policy.clone(),
        indexing_generation: policy.generation(),
    };
    assert!(task.is_current());
    policy.confirm(
        crate::infrastructure::drive_indexing::DriveIndexingSnapshot::new(
            vec![mtt_search_protocol::DriveIndexingEntry {
                drive_letter: 'R',
                enabled: false,
            }],
            true,
            vec![],
        )
        .unwrap(),
    );
    assert!(!task.is_current());
    assert_eq!(task.load().unwrap(), None);
}
