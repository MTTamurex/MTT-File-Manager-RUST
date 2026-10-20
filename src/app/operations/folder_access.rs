use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use crate::app::state::{FolderLoadErrorKind, ImageViewerApp};
use crate::infrastructure::windows::folder_access::{self, AccessError};

#[derive(Default)]
pub struct FolderAccessState {
    pending: Option<PendingGrant>,
    outcome: Option<(PathBuf, AccessError)>,
}

struct PendingGrant {
    path: PathBuf,
    receiver: Receiver<Result<(), AccessError>>,
}

impl ImageViewerApp {
    pub fn folder_access_pending(&self) -> bool {
        self.folder_access.pending.is_some()
    }

    pub fn folder_access_status(&self) -> Option<AccessError> {
        self.folder_access
            .outcome
            .as_ref()
            .and_then(|(path, error)| {
                matches_origin(path, Path::new(&self.navigation_state.current_path))
                    .then_some(*error)
            })
    }

    pub fn start_folder_access(&mut self) {
        if self.folder_access_pending() {
            return;
        }
        let Some(error) = self
            .folder_load_error
            .as_ref()
            .filter(|error| error.kind == FolderLoadErrorKind::AccessDenied)
        else {
            return;
        };
        let path = error.path.clone();
        let worker_path = path.clone();
        let owner_window = self.native_hwnd.map(|hwnd| hwnd.0 as isize);
        let ctx = self.ui_ctx.clone();
        let (sender, receiver) = mpsc::channel();
        self.folder_access.outcome = None;
        let worker = std::thread::Builder::new()
            .name("folder-access".into())
            .spawn(move || {
                let result = folder_access::request_access(&worker_path, owner_window);
                let _ = sender.send(result);
                ctx.request_repaint();
            });
        if worker.is_ok() {
            self.folder_access.pending = Some(PendingGrant { path, receiver });
        } else {
            self.folder_access.outcome = Some((path, AccessError::Failed(1)));
        }
    }

    pub(super) fn poll_folder_access(&mut self) {
        let Some(pending) = self.folder_access.pending.as_ref() else {
            return;
        };
        let result = match pending.receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(AccessError::Failed(1)),
        };
        let pending = self
            .folder_access
            .pending
            .take()
            .expect("pending grant exists");
        if let Err(error) = result {
            // Like a successful grant, a failure belongs to the path, not a
            // navigation generation that may change while UAC owns focus.
            self.folder_access.outcome = Some((pending.path, error));
            return;
        }

        self.directory_dirty_registry.mark_dirty(&pending.path);
        self.directory_cache.invalidate(&pending.path);
        self.invalidate_panel_tab_snapshots_for_path(&pending.path);

        // A grant changes the filesystem, not a particular load generation. A tab
        // restored during UAC has a new generation but may still show this error.
        if should_reload_granted_folder(
            &pending.path,
            Path::new(&self.navigation_state.current_path),
            self.folder_load_error.as_ref().map(|error| error.kind),
        ) {
            self.loaded_path.clear();
            self.load_folder(true);
        }
        if self.dual_panel_enabled {
            self.with_inactive_panel(|app| {
                if should_reload_granted_folder(
                    &pending.path,
                    Path::new(&app.navigation_state.current_path),
                    app.folder_load_error.as_ref().map(|error| error.kind),
                ) {
                    app.loaded_path.clear();
                    app.load_folder_for_inactive();
                }
            });
        }
    }
}

fn should_reload_granted_folder(
    granted_path: &Path,
    current_path: &Path,
    error_kind: Option<FolderLoadErrorKind>,
) -> bool {
    error_kind == Some(FolderLoadErrorKind::AccessDenied)
        && ImageViewerApp::normalize_for_match(granted_path)
            == ImageViewerApp::normalize_for_match(current_path)
}

fn matches_origin(path: &Path, current_path: &Path) -> bool {
    ImageViewerApp::normalize_for_match(path) == ImageViewerApp::normalize_for_match(current_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_matches_location_independently_of_navigation_generation() {
        let path = Path::new(r"C:\Folder");
        assert!(matches_origin(path, Path::new(r"c:\folder")));
        assert!(matches_origin(path, path));
        assert!(!matches_origin(path, Path::new(r"C:\Other")));
    }

    #[test]
    fn grant_reloads_visible_denied_folders_independently_of_load_generation() {
        let path = Path::new(r"C:\Folder");
        assert!(should_reload_granted_folder(
            path,
            Path::new(r"c:\folder"),
            Some(FolderLoadErrorKind::AccessDenied),
        ));
        assert!(!should_reload_granted_folder(
            path,
            Path::new(r"C:\Other"),
            Some(FolderLoadErrorKind::AccessDenied),
        ));
        assert!(!should_reload_granted_folder(path, path, None));
        assert!(!should_reload_granted_folder(
            path,
            path,
            Some(FolderLoadErrorKind::NotFound)
        ));
    }
}
