use crate::app::state::ImageViewerApp;
use crate::infrastructure::windows::network_drive_dialog::{self, DialogOutcome};
use std::io;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, TryRecvError};

pub(super) fn can_disconnect_network_drive(
    root: Option<&Path>,
    drive_type: Option<crate::infrastructure::windows::DriveType>,
) -> bool {
    drive_type == Some(crate::infrastructure::windows::DriveType::Remote)
        && root.is_some_and(crate::infrastructure::windows::is_drive_root_path)
}

#[derive(Default)]
pub(crate) struct NetworkDriveDialogState {
    completion: Option<Receiver<io::Result<DialogOutcome>>>,
    disconnect_root: Option<String>,
}

impl NetworkDriveDialogState {
    fn poll(&mut self) -> Option<io::Result<DialogOutcome>> {
        let result = match self.completion.as_ref()?.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => {
                Err(io::Error::other("Network dialog worker stopped"))
            }
        };
        self.completion = None;
        Some(result)
    }
}

impl ImageViewerApp {
    pub(crate) fn network_drive_dialog_pending(&self) -> bool {
        self.network_drive_dialog.completion.is_some()
    }

    pub(crate) fn start_network_drive_dialog(&mut self, disconnect_root: Option<String>) {
        if self.network_drive_dialog_pending() {
            return;
        }
        let owner = self.native_hwnd.map_or(0, |hwnd| hwnd.0 as isize);
        let root = disconnect_root.clone();
        let ctx = self.ui_ctx.clone();
        let (sender, receiver) = mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("network-drive-dialog".into())
            .spawn(move || {
                let _com = crate::infrastructure::windows::ComScope::sta();
                let result = match root {
                    Some(root) => network_drive_dialog::run_disconnect_dialog(owner, &root),
                    None => network_drive_dialog::run_map_dialog(owner),
                };
                let _ = sender.send(result);
                ctx.request_repaint();
            });
        if worker.is_ok() {
            self.network_drive_dialog.completion = Some(receiver);
            self.network_drive_dialog.disconnect_root = disconnect_root;
        } else {
            self.notifications
                .warning(rust_i18n::t!("operations.network_drive_dialog_failed"));
        }
    }

    pub(crate) fn poll_network_drive_dialog(&mut self) {
        let Some(result) = self.network_drive_dialog.poll() else {
            return;
        };
        // Native dialogs run on a worker thread; closing them does not reliably
        // reactivate the owner. Restore focus for success, cancellation and errors.
        self.restore_app_focus();
        let disconnected = self.network_drive_dialog.disconnect_root.take();
        match result {
            Ok(DialogOutcome::Completed) => {
                if let Some(root) = disconnected {
                    self.redirect_tabs_from_removed_drives(&[root]);
                }
                // Retain the refresh request if an older scan is still running.
                self.request_network_location_refresh();
            }
            Ok(DialogOutcome::Cancelled) => {}
            Err(error) => {
                log::warn!(
                    "[NETWORK-DRIVE] Native dialog failed (OS code {:?})",
                    error.raw_os_error()
                );
                self.notifications
                    .warning(rust_i18n::t!("operations.network_drive_dialog_failed"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_drive_dialog_disconnect_is_only_offered_for_remote_drive_roots() {
        use crate::infrastructure::windows::DriveType;
        assert!(can_disconnect_network_drive(
            Some(Path::new("Z:\\")),
            Some(DriveType::Remote)
        ));
        for kind in [DriveType::Fixed, DriveType::Removable, DriveType::Unknown] {
            assert!(!can_disconnect_network_drive(
                Some(Path::new("Z:\\")),
                Some(kind)
            ));
        }
        for path in [
            None,
            Some(Path::new("Z:\\folder")),
            Some(Path::new("\\\\server\\share")),
        ] {
            assert!(!can_disconnect_network_drive(path, Some(DriveType::Remote)));
        }
    }

    #[test]
    fn network_drive_dialog_completion_is_consumed_once() {
        for outcome in [DialogOutcome::Completed, DialogOutcome::Cancelled] {
            let (sender, receiver) = mpsc::channel();
            let mut state = NetworkDriveDialogState {
                completion: Some(receiver),
                ..Default::default()
            };
            assert!(state.poll().is_none());
            sender.send(Ok(outcome)).unwrap();
            assert!(state.poll().unwrap().is_ok());
            assert!(state.completion.is_none());
            assert!(state.poll().is_none());
        }
    }

    #[test]
    fn network_drive_dialog_worker_failure_releases_pending_state() {
        let (sender, receiver) = mpsc::channel();
        let mut state = NetworkDriveDialogState {
            completion: Some(receiver),
            ..Default::default()
        };
        drop(sender);
        assert!(state.poll().unwrap().is_err());
        assert!(state.completion.is_none());
    }
}
