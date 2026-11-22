use super::*;
use std::sync::{mpsc, Arc};

#[test]
fn shutdown_waits_for_final_snapshot_before_reporting_completion() {
    use crate::file_index::VolumeIndex;
    use crate::index_db::binary;

    crate::index_db::init_data_dir_for_tests();
    let mut index = VolumeIndex::empty('L');
    index.volume_id = r"\\?\Volume{abcdef01-2345-6789-abcd-ef0123456789}\".into();
    index.volume_serial = 123;
    index.journal_id = 7;
    index.last_usn = 42;
    index.hardlink_data_complete = true;
    index.reparse_data_complete = true;
    index.sizes_loaded = true;
    assert!(index.insert_record(10, "file.txt", 5, false, false));
    binary::save(&index).unwrap();

    let workers = Arc::new(IndexerThreads::default());
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    workers
        .spawn("blocked-final-snapshot".into(), move || {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            index.last_usn = 100;
            binary::save(&index).unwrap();
        })
        .unwrap();
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();

    let (pending_tx, pending_rx) = mpsc::sync_channel(1);
    let (stopped_tx, stopped_rx) = mpsc::channel();
    let shutdown_workers = workers.clone();
    let coordinator = thread::spawn(move || {
        shutdown_workers.shutdown(|| {
            let _ = pending_tx.try_send(());
        });
        stopped_tx.send(()).unwrap();
    });
    pending_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(stopped_rx.try_recv().is_err());
    assert_eq!(binary::load('L').unwrap().unwrap().1.last_usn, 42);

    release_tx.send(()).unwrap();
    stopped_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    coordinator.join().unwrap();
    assert_eq!(binary::load('L').unwrap().unwrap().1.last_usn, 100);
}

#[test]
fn discovery_cannot_spawn_an_untracked_indexer_during_shutdown() {
    let workers = Arc::new(IndexerThreads::default());
    let discovery_workers = workers.clone();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (spawn_result_tx, spawn_result_rx) = mpsc::channel();
    workers
        .spawn("blocked-discovery".into(), move || {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            let result = discovery_workers.spawn("late-indexer".into(), || {
                panic!("shutdown must reject new indexers")
            });
            spawn_result_tx.send(result.unwrap_err().kind()).unwrap();
        })
        .unwrap();
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();

    let (pending_tx, pending_rx) = mpsc::sync_channel(1);
    let shutdown_workers = workers.clone();
    let coordinator = thread::spawn(move || {
        shutdown_workers.shutdown(|| {
            let _ = pending_tx.try_send(());
        });
    });
    pending_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    release_tx.send(()).unwrap();
    assert_eq!(
        spawn_result_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap(),
        io::ErrorKind::Interrupted
    );
    coordinator.join().unwrap();
}

#[test]
fn empty_shutdown_reports_pending_and_permanently_closes_admission() {
    let workers = IndexerThreads::default();
    let mut reports = 0;
    workers.shutdown(|| reports += 1);
    assert_eq!(reports, 1);
    assert_eq!(
        workers
            .spawn("late-indexer".into(), || {})
            .unwrap_err()
            .kind(),
        io::ErrorKind::Interrupted
    );
}
