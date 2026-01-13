//! Folder preview worker for async thumbnail extraction using Windows Shell API
//!
//! Uses IShellItemImageFactory::GetImage to get native folder previews (sandwich effect).

use crate::infrastructure::windows::icons::get_folder_preview;
use eframe::egui;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

/// Data returned from folder preview worker
pub struct FolderPreviewData {
    pub path: PathBuf,
    pub rgba_data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Spawns a folder preview worker thread
///
/// # Arguments
/// * `rx` - Receiver for folder paths to process
/// * `tx` - Sender for processed preview data
/// * `ctx` - egui Context for repaint requests
pub fn spawn_folder_preview_worker(
    rx: Arc<Mutex<Receiver<PathBuf>>>,
    tx: Sender<FolderPreviewData>,
    ctx: egui::Context,
) {
    std::thread::spawn(move || {
        unsafe {
            // SAFETY: Initializing COM for this worker thread
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }

        loop {
            let path = match rx.lock().ok().and_then(|lock| lock.recv().ok()) {
                Some(p) => p,
                None => break,
            };

            // Get folder preview from Windows Shell
            // Get folder preview from Windows Shell
            match get_folder_preview(&path) {
                Ok((rgba_data, width, height)) => {
                    let _ = tx.send(FolderPreviewData {
                        path,
                        rgba_data,
                        width,
                        height,
                    });
                    ctx.request_repaint();
                }
                Err(_) => {
                    // Send empty data to signal failure/completion
                    // This signals the UI to stop the loading spinner
                    let _ = tx.send(FolderPreviewData {
                        path,
                        rgba_data: Vec::new(),
                        width: 0,
                        height: 0,
                    });
                    ctx.request_repaint();
                }
            }
        }

        unsafe {
            // SAFETY: Cleanup COM for this thread
            CoUninitialize();
        }
    });
}
