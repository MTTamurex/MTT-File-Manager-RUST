use crate::app::state::FolderLoadError;
use crate::domain::file_entry::FileEntry;
use crate::domain::network_path::unc_server_name;
use crate::infrastructure::windows::network_shares::list_network_shares;
use eframe::egui;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::Sender;

pub(super) fn try_load_network_server(
    path: &str,
    show_hidden: bool,
    generation: usize,
    current_generation: &AtomicUsize,
    entries_sender: &Sender<(usize, Vec<FileEntry>)>,
    failure_sender: &Sender<(usize, FolderLoadError)>,
    ctx: &egui::Context,
) -> bool {
    if unc_server_name(path).is_none() {
        return false;
    }
    let result = list_network_shares(path, show_hidden);
    if current_generation.load(Ordering::Relaxed) != generation {
        return true;
    }
    match result {
        Ok(entries) => {
            if !entries.is_empty() {
                let _ = entries_sender.send((generation, entries));
            }
            let _ = entries_sender.send((generation, Vec::new()));
        }
        Err(code) => {
            // A server container has no filesystem ACL to grant or vanished-folder
            // history to prune. Keep network errors on this view with their OS code.
            let error = std::io::Error::from_raw_os_error(code as i32);
            let _ = failure_sender.send((
                generation,
                FolderLoadError::other(PathBuf::from(path), error.to_string()),
            ));
        }
    }
    ctx.request_repaint();
    true
}
