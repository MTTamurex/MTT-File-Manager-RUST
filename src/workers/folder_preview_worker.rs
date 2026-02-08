//! Folder preview worker for async thumbnail extraction using Windows Shell API
//!
//! Uses IShellItemImageFactory::GetImage to get native folder previews (sandwich effect).

use crate::infrastructure::windows::icons::get_folder_preview;
use eframe::egui;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Instant;
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

        // PERFORMANCE: Set background priority to minimize HDD contention with video playback
        // This worker uses Windows Shell API to get folder previews - low priority I/O
        crate::infrastructure::io_priority::set_thread_priority(
            crate::infrastructure::io_priority::IOPriority::Background,
        );

        let mut last_repaint = Instant::now();
        while let Some(path) = rx.lock().ok().and_then(|lock| lock.recv().ok()) {
            // Skip cloud-only OneDrive folders — Shell API can block on network I/O
            if crate::infrastructure::onedrive::is_onedrive_path(&path)
                && !crate::infrastructure::onedrive::is_locally_available(&path)
            {
                let _ = tx.send(FolderPreviewData {
                    path,
                    rgba_data: Vec::new(),
                    width: 0,
                    height: 0,
                });
                throttle_repaint(&ctx, &mut last_repaint);
                continue;
            }

            // Get folder preview from Windows Shell
            match get_folder_preview(&path) {
                Ok((rgba_data, width, height)) => {
                    let _ = tx.send(FolderPreviewData {
                        path,
                        rgba_data,
                        width,
                        height,
                    });
                    throttle_repaint(&ctx, &mut last_repaint);
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
                    throttle_repaint(&ctx, &mut last_repaint);
                }
            }
        }

        unsafe {
            // SAFETY: Cleanup COM for this thread
            CoUninitialize();
        }
    });
}

fn throttle_repaint(ctx: &egui::Context, last_repaint: &mut Instant) {
    if last_repaint.elapsed().as_millis() >= 33 {
        ctx.request_repaint();
        *last_repaint = Instant::now();
    } else {
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }
}
