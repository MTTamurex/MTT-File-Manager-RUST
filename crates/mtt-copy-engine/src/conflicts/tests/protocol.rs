use super::*;

fn ask(
    progress: Arc<Progress>,
    kind: ConflictKind,
) -> std::thread::JoinHandle<io::Result<ConflictAction>> {
    std::thread::spawn(move || {
        progress.conflicts.ask(
            Conflict {
                id: 0,
                source: PathBuf::from("source/file.txt"),
                destination: PathBuf::from("target/file.txt"),
                kind,
            },
            &progress,
        )
    })
}

fn pending(progress: &Progress) -> Conflict {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(request) = progress.conflicts.pending() {
            return request;
        }
        assert!(Instant::now() < deadline, "Conflict was not published");
        std::thread::yield_now();
    }
}

#[test]
fn stale_or_inapplicable_decisions_never_authorize_a_conflict() {
    for paused in [false, true] {
        let progress = Arc::new(Progress::default());
        if paused {
            progress.pause();
        }
        let worker = ask(progress.clone(), ConflictKind::File);
        let request = pending(&progress);
        assert!(!progress
            .conflicts
            .respond(request.id, ConflictAction::Merge, true));
        assert!(!progress.conflicts.respond(
            request.id.wrapping_add(1),
            ConflictAction::Replace,
            true
        ));
        assert!(!worker.is_finished());
        progress.cancel();
        assert_eq!(
            worker.join().unwrap().unwrap_err().kind(),
            io::ErrorKind::Interrupted
        );
        assert!(progress.conflicts.pending().is_none());
        assert!(!progress
            .conflicts
            .respond(request.id, ConflictAction::Replace, true));
    }
}

#[test]
fn remembered_answers_apply_only_to_the_same_kind_and_operation() {
    let progress = Arc::new(Progress::default());
    let worker = ask(progress.clone(), ConflictKind::File);
    let request = pending(&progress);
    assert!(progress
        .conflicts
        .respond(request.id, ConflictAction::Replace, true));
    assert_eq!(worker.join().unwrap().unwrap(), ConflictAction::Replace);
    let (sender, receiver) = std::sync::mpsc::channel();
    let control = progress.clone();
    let worker = std::thread::spawn(move || {
        let action = control.conflicts.ask(
            Conflict {
                id: 0,
                source: PathBuf::from("source/other.txt"),
                destination: PathBuf::from("target/other.txt"),
                kind: ConflictKind::File,
            },
            &control,
        );
        let _ = sender.send(action);
    });
    let answer = receiver.recv_timeout(Duration::from_secs(3));
    if answer.is_err() {
        progress.cancel();
    }
    worker.join().unwrap();
    assert_eq!(answer.unwrap().unwrap(), ConflictAction::Replace);
    let worker = ask(progress.clone(), ConflictKind::Directory);
    let request = pending(&progress);
    assert!(progress
        .conflicts
        .respond(request.id, ConflictAction::Skip, false));
    assert_eq!(worker.join().unwrap().unwrap(), ConflictAction::Skip);
    let fresh = Arc::new(Progress::default());
    let worker = ask(fresh.clone(), ConflictKind::File);
    let request = pending(&fresh);
    fresh
        .conflicts
        .respond(request.id, ConflictAction::Cancel, false);
    assert_eq!(
        worker.join().unwrap().unwrap_err().kind(),
        io::ErrorKind::Interrupted
    );
}
