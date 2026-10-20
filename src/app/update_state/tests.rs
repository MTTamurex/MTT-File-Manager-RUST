use super::*;

fn receive(state: &mut UpdateState, event: UpdateEvent) {
    let (tx, rx) = mpsc::channel();
    state.receiver = Some(rx);
    tx.send(event)
        .unwrap_or_else(|_| panic!("test receiver disconnected"));
    state.poll(&egui::Context::default());
}

#[test]
fn automatic_installation_is_opt_in() {
    let mut state = UpdateState::default();
    assert!(!state.auto_install);
    state.automatic_check = true;
    receive(
        &mut state,
        UpdateEvent::Checked(Ok(Some(Release {
            version: semver::Version::new(0, 3, 0),
            notes: "Release notes".into(),
            url: String::new(),
            size: 1,
            digest: "0".repeat(64),
        }))),
    );
    assert!(state.show_dialog);
    assert!(!state.downloading);
}

#[test]
fn completed_download_waits_for_safe_handoff() {
    let mut state = UpdateState::default();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().to_owned();
    receive(
        &mut state,
        UpdateEvent::Downloaded(Ok(DownloadedUpdate {
            directory,
            digest: "0".repeat(64),
        })),
    );
    assert!(state.waiting());
    assert!(!state.exiting);
    assert!(!path.join("install").exists());
    state.postpone();
    assert!(!state.waiting());
    assert!(!path.exists());
}

#[test]
fn disconnected_worker_can_be_retried_without_leaving_busy_state() {
    let mut state = UpdateState::default();
    let (tx, rx) = mpsc::channel();
    state.receiver = Some(rx);
    state.downloading = true;
    drop(tx);
    state.poll(&egui::Context::default());
    assert!(!state.downloading);
    assert!(state.error.is_some());
    assert!(state.receiver.is_none());
}

#[test]
fn fresh_check_dismisses_stale_release_details_when_no_update_is_available() {
    let mut state = UpdateState::default();
    state.show_dialog = true;
    receive(&mut state, UpdateEvent::Checked(Ok(None)));
    assert!(!state.show_dialog);
    assert!(state.up_to_date);
    assert!(state.release.is_none());
}

#[test]
fn failed_or_postponed_handoff_does_not_poison_the_next_attempt() {
    for fail in [false, true] {
        let mut state = UpdateState::default();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().to_owned();
        state.ready = Some(DownloadedUpdate {
            directory,
            digest: "0".repeat(64),
        });
        let child = std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Sleep -Seconds 30",
            ])
            .spawn()
            .unwrap();
        state.helper = Some(updater::HelperProcess::new(child));
        if fail {
            state.fail_handoff("handoff failed".into());
        } else {
            state.postpone();
        }
        assert!(state.helper.is_none());
        assert!(!state.waiting());
        assert!(!path.exists());
        receive(
            &mut state,
            UpdateEvent::Downloaded(Ok(DownloadedUpdate {
                directory: tempfile::tempdir().unwrap(),
                digest: "0".repeat(64),
            })),
        );
        assert!(state.waiting());
        assert!(state.helper.is_none());
    }
}
