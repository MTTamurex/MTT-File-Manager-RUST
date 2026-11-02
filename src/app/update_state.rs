use crate::infrastructure::updater::{self, Release};
use eframe::egui;
use std::sync::mpsc;

#[cfg(test)]
mod tests;

#[derive(Default)]
pub(crate) struct UpdateState {
    initialized: bool,
    pub check_on_startup: bool,
    pub checking: bool,
    pub show_dialog: bool,
    pub up_to_date: bool,
    pub error: Option<String>,
    pub release: Option<Release>,
    pub notes_cache: egui_commonmark::CommonMarkCache,
    receiver: Option<mpsc::Receiver<Result<Option<Release>, String>>>,
}

impl UpdateState {
    pub(crate) fn check(&mut self, ctx: &egui::Context) {
        if self.checking {
            return;
        }
        self.error = None;
        self.up_to_date = false;
        self.checking = true;
        let (tx, rx) = mpsc::channel();
        self.receiver = Some(rx);
        let ctx = ctx.clone();
        if crate::spawn_named("update-check", move || {
            let _ = tx.send(updater::check());
            ctx.request_repaint();
        })
        .is_err()
        {
            self.checking = false;
            self.receiver = None;
            self.error = Some("Could not start the update check".into());
        }
    }

    pub(crate) fn open_release(&self, ctx: &egui::Context) {
        if let Some(release) = &self.release {
            ctx.open_url(egui::OpenUrl::new_tab(&release.url));
        }
    }

    pub(crate) fn postpone(&mut self) {
        self.show_dialog = false;
        self.error = None;
    }

    fn poll(&mut self) {
        let event = self.receiver.as_ref().map(|rx| rx.try_recv());
        match event {
            Some(Ok(result)) => {
                self.receiver = None;
                self.checking = false;
                match result {
                    Ok(release) => {
                        self.up_to_date = release.is_none();
                        self.release = release;
                        self.notes_cache = egui_commonmark::CommonMarkCache::default();
                        self.show_dialog = self.release.is_some();
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                self.receiver = None;
                self.checking = false;
                self.error = Some("Update worker stopped unexpectedly".into());
            }
            _ => {}
        }
    }
}

impl super::ImageViewerApp {
    pub(crate) fn poll_updates(&mut self, ctx: &egui::Context) {
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
            if self.updates.check_on_startup {
                self.updates.check(ctx);
            }
        }
        self.updates.poll();
    }
}
