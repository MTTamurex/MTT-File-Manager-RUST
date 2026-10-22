//! Close the reply channel before waking the UI, even during unwinding.
use super::{FolderDates, ReadResult};
use eframe::egui;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

struct Completion {
    sender: Option<mpsc::Sender<Vec<ReadResult>>>,
    ctx: egui::Context,
}

impl Drop for Completion {
    fn drop(&mut self) {
        // If reading panicked, the receiver must observe Disconnected on the
        // very repaint we request. Waking it before dropping the sender races
        // with the UI seeing Empty and then sleeping indefinitely.
        drop(self.sender.take());
        self.ctx.request_repaint();
    }
}

pub(super) fn run(paths: Vec<PathBuf>, sender: mpsc::Sender<Vec<ReadResult>>, ctx: egui::Context) {
    run_with(paths, sender, ctx, read_dates);
}

fn run_with(
    paths: Vec<PathBuf>,
    sender: mpsc::Sender<Vec<ReadResult>>,
    ctx: egui::Context,
    mut read: impl FnMut(&Path) -> std::io::Result<FolderDates>,
) {
    let mut completion = Completion {
        sender: Some(sender),
        ctx,
    };
    let results = paths
        .into_iter()
        .map(|path| {
            let dates = read(&path).ok();
            ReadResult { path, dates }
        })
        .collect();
    if let Some(sender) = completion.sender.take() {
        let _ = sender.send(results);
    }
}

pub(super) fn read_dates(path: &Path) -> std::io::Result<FolderDates> {
    let metadata = std::fs::metadata(path)?;
    // A real directory may be named folder.zip. A virtual entry inside an
    // actual archive cannot pass this filesystem check, done off the UI.
    if !metadata.is_dir() {
        return Err(std::io::Error::other("Folder is no longer a directory"));
    }
    let seconds = |value: std::io::Result<std::time::SystemTime>| {
        value
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_secs())
    };
    Ok(FolderDates {
        modified: seconds(metadata.modified()).unwrap_or(0),
        created: seconds(metadata.created()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn panic_disconnects_the_channel_before_the_repaint_callback() {
        let ctx = egui::Context::default();
        // Settle initial egui/font repaints so the next request invokes its callback.
        for _ in 0..4 {
            let _ = ctx.run_ui(egui::RawInput::default(), |_| {});
        }
        let (sender, receiver) = mpsc::channel();
        let observed = Arc::new(Mutex::new(Vec::new()));
        let receiver = Mutex::new(receiver);
        let callback_observed = observed.clone();
        ctx.set_request_repaint_callback(move |_| {
            callback_observed.lock().unwrap().push(matches!(
                receiver.lock().unwrap().try_recv(),
                Err(mpsc::TryRecvError::Disconnected)
            ));
        });
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_with(vec![PathBuf::from(r"C:\Folder")], sender, ctx, |_| {
                panic!("synthetic metadata reader failure")
            });
        }));
        assert!(result.is_err());
        assert_eq!(*observed.lock().unwrap(), [true]);
    }

    #[test]
    fn only_real_directories_produce_folder_dates() {
        let dir = tempfile::tempdir().unwrap();
        let child = dir.path().join("folder.zip").join("Child");
        std::fs::create_dir_all(&child).unwrap();
        assert!(read_dates(&child).is_ok());
        let archive = dir.path().join("archive.zip");
        std::fs::write(&archive, b"fixture").unwrap();
        assert!(read_dates(&archive).is_err());
        assert!(read_dates(&archive.join("Child")).is_err());
    }
}
