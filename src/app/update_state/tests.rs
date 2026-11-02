use super::*;

fn release() -> Release {
    Release {
        version: semver::Version::new(0, 3, 0),
        notes: "Release notes".into(),
        url: "https://example.invalid/releases/tag/v0.3.0".into(),
    }
}

fn receive(state: &mut UpdateState, result: Result<Option<Release>, String>) {
    let (tx, rx) = mpsc::channel();
    state.receiver = Some(rx);
    tx.send(result).unwrap();
    state.poll();
}

#[test]
fn completed_check_only_shows_release_details_without_opening_a_browser() {
    let mut state = UpdateState::default();
    let ctx = egui::Context::default();
    let output = ctx.run_ui(egui::RawInput::default(), |_ui| {
        receive(&mut state, Ok(Some(release())));
    });
    assert!(state.show_dialog);
    assert!(!state.checking);
    assert!(state.release.is_some());
    assert!(output.platform_output.commands.is_empty());
}

#[test]
fn explicit_action_opens_the_release_page_in_the_browser() {
    let state = UpdateState {
        release: Some(release()),
        ..Default::default()
    };
    let ctx = egui::Context::default();
    let output = ctx.run_ui(egui::RawInput::default(), |ui| {
        state.open_release(ui.ctx());
    });
    assert_eq!(output.platform_output.commands.len(), 1);
    assert!(matches!(
        &output.platform_output.commands[0],
        egui::OutputCommand::OpenUrl(url) if url.url == release().url && url.new_tab
    ));
}

#[test]
fn postpone_keeps_release_details_available_without_an_install_handoff() {
    let mut state = UpdateState {
        release: Some(release()),
        show_dialog: true,
        ..Default::default()
    };
    state.postpone();
    assert!(!state.show_dialog);
    assert!(state.release.is_some());
}

#[test]
fn disconnected_worker_does_not_leave_busy_state() {
    let mut state = UpdateState {
        checking: true,
        ..Default::default()
    };
    let (tx, rx) = mpsc::channel();
    state.receiver = Some(rx);
    drop(tx);
    state.poll();
    assert!(!state.checking);
    assert!(state.error.is_some());
    assert!(state.receiver.is_none());
}

#[test]
fn fresh_check_dismisses_stale_release_details_when_no_update_is_available() {
    let mut state = UpdateState {
        release: Some(release()),
        show_dialog: true,
        ..Default::default()
    };
    receive(&mut state, Ok(None));
    assert!(!state.show_dialog);
    assert!(state.up_to_date);
    assert!(state.release.is_none());
}
