use crate::app::state::ImageViewerApp;
use std::sync::mpsc::{self, Receiver, TryRecvError};

#[derive(Default)]
pub(crate) struct NetworkLocationWizardState {
    completion: Option<Receiver<bool>>,
    refresh_requested: bool,
}

impl NetworkLocationWizardState {
    fn poll_completion(&mut self) -> Option<bool> {
        let receiver = self.completion.as_ref()?;
        let success = match receiver.try_recv() {
            Ok(success) => success,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => false,
        };
        self.completion = None;
        self.refresh_requested |= success;
        Some(success)
    }

    fn take_refresh_if_ready(&mut self, drive_scan_pending: bool) -> bool {
        if drive_scan_pending {
            return false;
        }
        std::mem::take(&mut self.refresh_requested)
    }
}

impl ImageViewerApp {
    pub(crate) fn network_location_wizard_pending(&self) -> bool {
        self.network_location_wizard.completion.is_some()
    }

    pub(crate) fn start_network_location_wizard(&mut self) {
        if self.network_location_wizard_pending() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let ctx = self.ui_ctx.clone();
        let worker = std::thread::Builder::new()
            .name("network-location-wizard".into())
            .spawn(move || {
                let result = crate::infrastructure::windows::network_location_wizard::run_network_location_wizard();
                if let Err(error) = &result {
                    log::warn!("[NETWORK-LOCATION] Wizard launch/wait failed (OS code {:?})", error.raw_os_error());
                }
                let _ = sender.send(result.is_ok());
                ctx.request_repaint();
            });
        if worker.is_ok() {
            self.network_location_wizard.completion = Some(receiver);
        } else {
            self.notifications
                .warning(rust_i18n::t!("operations.network_location_wizard_failed"));
        }
    }

    pub(super) fn poll_network_location_wizard(&mut self) {
        if self.network_location_wizard.poll_completion() == Some(false) {
            self.notifications
                .warning(rust_i18n::t!("operations.network_location_wizard_failed"));
        }
        // A scan started before the wizard closed may not contain the new shortcut.
        // Retain the request until that scan finishes, then perform a fresh one.
        if self
            .network_location_wizard
            .take_refresh_if_ready(self.drive_state.drive_scan_pending)
        {
            self.reload_drive_list_async();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_location_wizard_completion_requests_one_refresh_after_an_existing_scan() {
        let (sender, receiver) = mpsc::channel();
        let mut state = NetworkLocationWizardState {
            completion: Some(receiver),
            ..Default::default()
        };
        assert_eq!(state.poll_completion(), None);
        sender.send(true).unwrap();
        assert_eq!(state.poll_completion(), Some(true));
        assert!(state.completion.is_none());
        assert!(!state.take_refresh_if_ready(true));
        assert!(state.take_refresh_if_ready(false));
        assert!(!state.take_refresh_if_ready(false));
    }

    #[test]
    fn network_location_wizard_worker_failure_releases_the_pending_state() {
        let (sender, receiver) = mpsc::channel();
        let mut state = NetworkLocationWizardState {
            completion: Some(receiver),
            ..Default::default()
        };
        drop(sender);
        assert_eq!(state.poll_completion(), Some(false));
        assert!(state.completion.is_none());
        assert!(!state.take_refresh_if_ready(false));
    }
}
