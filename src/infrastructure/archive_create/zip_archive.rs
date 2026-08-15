//! ZIP archive creation via the `zip` crate (deflate).

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;

use super::{
    cancelled_error, is_cancelled, update_progress, CompressionCancelFlag, PendingEntry,
    ProgressReader, SharedCompressionProgress,
};

fn zip_error(error: zip::result::ZipError) -> io::Error {
    io::Error::other(error.to_string())
}

/// Read buffer for copying source files into the zip stream.
/// Tests shrink it: cancellation tests must wait out one deflate chunk, and
/// unoptimized debug deflate is orders of magnitude slower than release.
#[cfg(not(test))]
const COPY_BUFFER_BYTES: usize = 64 * 1024;
#[cfg(test)]
const COPY_BUFFER_BYTES: usize = 4 * 1024;

pub(super) fn write_archive(
    entries: &[PendingEntry],
    dest: &Path,
    archive_name: &str,
    total: usize,
    total_bytes: u64,
    progress: &SharedCompressionProgress,
    cancel: &CompressionCancelFlag,
) -> io::Result<()> {
    let file = File::create(dest)?;
    let mut writer = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();

    let mut processed = 0usize;
    let mut bytes_done = 0u64;
    for entry in entries {
        if is_cancelled(cancel) {
            return Err(cancelled_error());
        }
        if entry.is_dir {
            writer
                .add_directory(format!("{}/", entry.name), options)
                .map_err(zip_error)?;
            continue;
        }

        update_progress(
            progress,
            archive_name,
            &entry.name,
            processed,
            total,
            bytes_done,
            total_bytes,
        );
        let mut source = ProgressReader::new(
            File::open(&entry.source)?,
            processed,
            bytes_done,
            progress.clone(),
            cancel.clone(),
        );

        writer
            .start_file(entry.name.as_str(), options)
            .map_err(zip_error)?;

        let mut buffer = [0u8; COPY_BUFFER_BYTES];
        loop {
            let read = source.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            writer.write_all(&buffer[..read])?;
        }

        processed += 1;
        bytes_done += entry.size;
    }

    writer.finish().map_err(zip_error)?;
    Ok(())
}
