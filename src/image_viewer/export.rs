//! Capture the destination accepted in the save dialog before decoding starts.
use super::loader::{normalize_export_path, DecodedFrame, ExportImageFormat};
use super::save::{self, FileSnapshot};
use std::io;
use std::path::PathBuf;

pub(super) struct ExportDestination {
    path: PathBuf,
    format: ExportImageFormat,
    expected: Option<FileSnapshot>,
}

impl ExportDestination {
    pub(super) fn confirm(selected: PathBuf, format: ExportImageFormat) -> io::Result<Self> {
        let path = normalize_export_path(&selected, format);
        let expected = match save::capture_file_snapshot(&path) {
            Ok(snapshot) if path == selected => Some(snapshot),
            Ok(_) => return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "The normalized export destination already exists; select its exact filename to confirm replacement",
            )),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        Ok(Self {
            path,
            format,
            expected,
        })
    }

    pub(super) fn write(self, frame: DecodedFrame) -> io::Result<PathBuf> {
        save::save_export_atomically(frame, self.format, &self.path, self.expected)?;
        Ok(self.path)
    }
}

#[cfg(test)]
mod tests;
