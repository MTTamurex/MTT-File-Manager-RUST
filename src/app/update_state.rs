use crate::infrastructure::updater::{self, DownloadedUpdate, Release};
use eframe::egui;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    mpsc, Arc,
};

#[cfg(test)]
mod tests;

pub(crate) enum UpdateEvent {
    Checked(Result<Option<Release>, String>),
    Downloaded(Result<DownloadedUpdate, String>),
}

#[derive(Default)]
pub(crate) struct UpdateState {
    initialized: bool,
    pub check_on_startup: bool,
    pub auto_install: bool,
    pub checking: bool,
    pub downloading: bool,
    pub show_dialog: bool,
    pub up_to_date: bool,
    pub error: Option<String>,
    pub release: Option<Release>,
    pub progress: Arc<AtomicU64>,
    receiver: Option<mpsc::Receiver<UpdateEvent>>,
    helper: Option<updater::HelperProcess>,
    ready: Option<DownloadedUpdate>,
    pub exiting: bool,
    automatic_check: bool,
}

impl UpdateState {
    pub(crate) fn check(&mut self, ctx: &egui::Context, automatic: bool) {
        if self.checking || self.downloading || self.ready.is_some() {
            return;
        }
        self.error = None;
        self.up_to_date = false;
        self.checking = true;
        self.automatic_check = automatic;
        let (tx, rx) = mpsc::channel();
        self.receiver = Some(rx);
        let ctx = ctx.clone();
        if crate::spawn_named("update-check", move || {
            let _ = tx.send(UpdateEvent::Checked(updater::check()));
            ctx.request_repaint();
        })
        .is_err()
        {
            self.checking = false;
            self.receiver = None;
            self.error = Some("Could not start the update check".into());
        }
    }

    pub(crate) fn download(&mut self, ctx: &egui::Context) {
        if self.checking || self.downloading || self.ready.is_some() {
            return;
        }
        let Some(release) = self.release.clone() else {
            return;
        };
        if updater::platform::elevated() {
            self.error =
                Some("Restart MTT without administrator privileges before updating".into());
            return;
        }
        self.error = None;
        self.downloading = true;
        self.progress.store(0, Ordering::Relaxed);
        let progress = self.progress.clone();
        let (tx, rx) = mpsc::channel();
        self.receiver = Some(rx);
        let ctx = ctx.clone();
        if crate::spawn_named("update-download", move || {
            let result = updater::download(&release, |count, _| {
                progress.store(count, Ordering::Relaxed);
            });
            let _ = tx.send(UpdateEvent::Downloaded(result));
            ctx.request_repaint();
        })
        .is_err()
        {
            self.downloading = false;
            self.receiver = None;
            self.error = Some("Could not start the update download".into());
        }
    }

    pub(crate) fn waiting(&self) -> bool {
        self.ready.is_some()
    }

    pub(crate) fn postpone(&mut self) {
        self.show_dialog = false;
        self.helper = None;
        self.ready = None;
        self.error = None;
    }

    fn fail_handoff(&mut self, error: String) {
        // Stop the waiting helper before removing its files; the next attempt
        // must never inherit a child associated with a previous download.
        self.helper = None;
        self.ready = None;
        self.error = Some(error);
    }

    fn poll(&mut self, ctx: &egui::Context) {
        let event = self.receiver.as_ref().map(|rx| rx.try_recv());
        match event {
            Some(Ok(event)) => {
                self.receiver = None;
                self.checking = false;
                self.downloading = false;
                match event {
                    UpdateEvent::Checked(Ok(release)) => {
                        self.up_to_date = release.is_none();
                        self.release = release;
                        self.show_dialog = self.release.is_some();
                        if self.release.is_some() {
                            if self.automatic_check
                                && self.auto_install
                                && !updater::platform::elevated()
                            {
                                self.download(ctx);
                            }
                        }
                    }
                    UpdateEvent::Checked(Err(error)) | UpdateEvent::Downloaded(Err(error)) => {
                        self.error = Some(error)
                    }
                    UpdateEvent::Downloaded(Ok(update)) => self.ready = Some(update),
                }
            }
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                self.receiver = None;
                self.checking = false;
                self.downloading = false;
                self.error = Some("Update worker stopped unexpectedly".into());
            }
            _ => {}
        }
        if self.downloading || self.ready.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
    }
}

impl super::ImageViewerApp {
    pub(crate) fn poll_updates(&mut self, ctx: &egui::Context, organizer_commands_pending: bool) {
        if self.startup_tick < 2 {
            return;
        }
        if !self.updates.initialized {
            self.updates.initialized = true;
            self.updates.check_on_startup = self
                .app_state_db
                .get_preference("updates_check_on_startup")
                .as_deref()
                != Some("false");
            self.updates.auto_install = self
                .app_state_db
                .get_preference("updates_auto_install")
                .as_deref()
                == Some("true");
            let skip_startup = std::env::var_os("MTT_SKIP_STARTUP_UPDATE").is_some();
            // Windows environment mutation is thread-safe; consume the one-shot
            // marker so subsequently spawned MTT windows do not inherit it.
            std::env::remove_var("MTT_SKIP_STARTUP_UPDATE");
            if self.updates.check_on_startup && !skip_startup {
                self.updates.check(ctx, true);
            }
        }
        self.updates.poll(ctx);
        if self.updates.exiting || self.updates.ready.is_none() {
            return;
        }
        // Sample commands before draining events, then check again: a completed
        // resume/set-rules command must not hide behind stale paused UI state.
        if organizer_commands_pending || self.organizer_state.manager.has_pending_commands() {
            return;
        }
        // A running watcher can enqueue new moves between the idle check and exit.
        // Require its existing pause mechanism instead of racing that producer.
        if self.organizer_state.rules.iter().any(|rule| {
            use crate::infrastructure::organizer::OrganizerRuleStatus::{Disabled, Paused};
            let status = self.organizer_state.rule_statuses.get(&rule.id);
            status != Some(&Paused) && (rule.enabled || !matches!(status, None | Some(Disabled)))
        }) {
            self.updates.error = Some("Pause enabled organizer rules before updating".into());
            return;
        }
        // These counters include archive/image operations and organizer work.
        if self.file_operation_state.file_ops_in_progress > 0
            || self.file_operation_state.batch_rename_progress.is_some()
            || self.file_operation_state.pending_iso_mount.is_some()
            || !self.organizer_state.active_operation_ids.is_empty()
        {
            return;
        }
        if updater::platform::elevated() {
            self.updates.fail_handoff(
                "Restart MTT without administrator privileges before updating".into(),
            );
            return;
        }
        match updater::platform::other_instances(std::process::id()) {
            Ok(true) => {
                self.updates.error =
                    Some("Close other MTT windows and viewers to continue updating".into());
                return;
            }
            Err(error) => {
                self.updates.error = Some(error);
                return;
            }
            Ok(false) => {}
        }
        self.updates.error = None;
        if self.updates.helper.is_none() {
            match self.updates.ready.as_ref().unwrap().spawn_helper() {
                Ok(child) => self.updates.helper = Some(child),
                Err(error) => {
                    self.updates.fail_handoff(error);
                }
            }
            return;
        }
        match self
            .updates
            .helper
            .as_mut()
            .unwrap()
            .ready(self.updates.ready.as_ref().unwrap().directory.path())
        {
            Ok(true) => {}
            Ok(false) => return,
            Err(error) => {
                self.updates.fail_handoff(error);
                return;
            }
        }
        self.sync_to_tab();
        self.save_preferences();
        if !self.force_save_preferences() {
            self.updates
                .fail_handoff("Could not save the session; update postponed".into());
            return;
        }
        if std::fs::write(
            self.updates
                .ready
                .as_ref()
                .unwrap()
                .directory
                .path()
                .join("install"),
            b"install",
        )
        .is_err()
        {
            self.updates
                .fail_handoff("Could not authorize the update handoff".into());
            return;
        }
        let update = self.updates.ready.take().unwrap();
        let _ = update.directory.keep();
        drop(self.updates.helper.take().unwrap().detach());
        self.updates.exiting = true;
        self.tray_exit_requested = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}
