use super::*;
use crossbeam_channel::{bounded, Receiver};
use std::{
    io::{self, Write},
    path::PathBuf,
};

pub(crate) enum Destination {
    Clipboard,
    File { path: PathBuf, overwrite: bool },
}

pub(crate) struct Completed {
    pub clipboard: Option<String>,
    pub skipped: usize,
    pub count: usize,
}

pub(crate) struct Job {
    pub progress: Arc<Progress>,
    pub result: Receiver<Result<Completed, ExportError>>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.progress.cancelled.store(true, Ordering::Relaxed);
    }
}

pub(crate) fn start(
    options: Options,
    destination: Destination,
    ctx: eframe::egui::Context,
) -> Result<Job, ExportError> {
    let progress = Arc::new(Progress::default());
    let worker_progress = progress.clone();
    let (sender, result) = bounded(1);
    crate::spawn_named("folder-listing-export", move || {
        let result = run(&options, destination, &worker_progress);
        let _ = sender.send(result);
        ctx.request_repaint();
    })
    .map_err(|_| ExportError::Worker)?;
    Ok(Job { progress, result })
}

struct ClipboardBuffer(Vec<u8>);
impl Write for ClipboardBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > 32 * 1024 * 1024 {
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "clipboard limit",
            ));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn run(
    options: &Options,
    destination: Destination,
    progress: &Progress,
) -> Result<Completed, ExportError> {
    let _com = crate::infrastructure::windows::metadata::utils::ComGuard::new()
        .ok_or(ExportError::Worker)?;
    let listing = super::collect::collect(options, progress)?;
    let count = listing.rows.iter().filter(|row| row.included).count();
    let clipboard = match destination {
        Destination::Clipboard => {
            let mut buffer = ClipboardBuffer(Vec::new());
            super::render::render(&mut buffer, options, &listing, progress)?;
            progress.check()?;
            // A BOM is an encoding signature for files, not part of clipboard text.
            if matches!(options.format, Format::Csv | Format::Tsv) {
                buffer.0.drain(..3);
            }
            Some(String::from_utf8(buffer.0).map_err(|_| ExportError::Write)?)
        }
        Destination::File { path, overwrite } => {
            // Temporary output is a sibling: same-volume rename, no partially written report.
            let parent = path.parent().ok_or(ExportError::Write)?;
            let mut temp =
                tempfile::NamedTempFile::new_in(parent).map_err(|_| ExportError::Write)?;
            {
                let mut output = io::BufWriter::new(temp.as_file_mut());
                super::render::render(&mut output, options, &listing, progress)?;
                output.flush().map_err(|_| ExportError::Write)?;
            }
            temp.as_file().sync_all().map_err(|_| ExportError::Write)?;
            progress.check()?;
            if overwrite {
                temp.persist(&path).map_err(|_| ExportError::Write)?;
            } else {
                temp.persist_noclobber(&path).map_err(|error| {
                    if error.error.kind() == io::ErrorKind::AlreadyExists {
                        ExportError::Exists
                    } else {
                        ExportError::Write
                    }
                })?;
            }
            None
        }
    };
    Ok(Completed {
        clipboard,
        skipped: listing.skipped,
        count,
    })
}
