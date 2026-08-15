//! 7Z archive creation via `sevenz-rust2` (LZMA2, lzma-rust2 encoder).
//!
//! Engine selection:
//! - Multithreaded LZMA2 (4 MiB independent chunks, cores/2 workers capped
//!   at 8) when the machine has >= 4 cores, the selection's average entry is
//!   large enough for per-entry thread pools to pay off, and the total size
//!   is within [`MT_MAX_TOTAL_BYTES`]. The crate's dispatch queue has no
//!   backpressure, so while an MT job runs, worst-case RAM grows with the
//!   selection and byte progress can reach the total before the last chunks
//!   finish encoding (bounded by the queued data, drained in order).
//! - Otherwise single-threaded: bounded RAM and byte progress identical to
//!   real compression.
//!
//! Entries are written non-solid (one block per file, `push_archive_entry`),
//! keeping memory bounded per file and giving a cancellation point between
//! entries. The encoder reads inline through `ProgressReader`, so byte-level
//! progress and mid-file cancellation work without touching the crate.
//!
//! Pausing the feeder to throttle the MT encoder is NOT possible: completed
//! chunks are only drained inside the crate's write path, so a stalled
//! reader deadlocks the worker pool (verified against lzma-rust2 0.16.5).

use std::fs::File;
use std::io;
use std::path::Path;

use sevenz_rust2::encoder_options::Lzma2Options;

use super::{
    cancelled_error, is_cancelled, update_progress, CompressionCancelFlag, PendingEntry,
    ProgressReader, SharedCompressionProgress,
};

fn seven_zip_error(error: sevenz_rust2::Error) -> io::Error {
    io::Error::other(error.to_string())
}

/// Compression level for LZMA2 (0-9). Level 3 uses the fast HC4 match finder;
/// levels 5+ (BT4/Normal mode) measured ~4x slower for only ~9% better ratio.
const LZMA2_LEVEL: u32 = 3;

/// Uncompressed size of each independent MT chunk. Smaller chunks improve
/// parallelism at the cost of ratio; 4 MiB measured ratio-neutral here.
const LZMA2_MT_CHUNK_BYTES: u64 = 4 << 20;

/// Selections up to this total size are encoded multithreaded. Above it the
/// job runs single-threaded: the crate's dispatch queue has no backpressure,
/// so an MT job can buffer up to the whole selection in RAM.
const MT_MAX_TOTAL_BYTES: u64 = 1 << 30;

#[cfg(test)]
static TEST_FORCE_SINGLE_THREAD: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Test-only: force the single-threaded encoder so cancellation tests stay
/// deterministic and cheap regardless of MT scheduling.
#[cfg(test)]
pub(crate) fn set_test_force_single_thread(force: bool) {
    TEST_FORCE_SINGLE_THREAD.store(force, std::sync::atomic::Ordering::Relaxed);
}

/// Below this average entry size the per-entry MT thread pools cost more
/// than they save (each small file compresses as a single chunk anyway).
const MT_MIN_AVG_ENTRY_BYTES: u64 = 1 << 20;

/// Worker threads for the MT encoder given `cores`; `None` selects
/// single-threaded encoding.
fn mt_threads_for(cores: usize, total_bytes: u64, file_count: usize) -> Option<u32> {
    if cores < 4 || file_count == 0 || total_bytes > MT_MAX_TOTAL_BYTES {
        return None;
    }
    if total_bytes / file_count as u64 >= MT_MIN_AVG_ENTRY_BYTES {
        Some((cores / 2).clamp(2, 8) as u32)
    } else {
        None
    }
}

fn encoder_threads(total_bytes: u64, file_count: usize) -> Option<u32> {
    #[cfg(test)]
    if TEST_FORCE_SINGLE_THREAD.load(std::sync::atomic::Ordering::Relaxed) {
        return None;
    }
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    mt_threads_for(cores, total_bytes, file_count)
}

/// Picks the LZMA2 configuration for a job of `total_bytes`/`file_count`.
fn lzma2_configuration(total_bytes: u64, file_count: usize) -> Lzma2Options {
    match encoder_threads(total_bytes, file_count) {
        Some(threads) => Lzma2Options::from_level_mt(LZMA2_LEVEL, threads, LZMA2_MT_CHUNK_BYTES),
        None => Lzma2Options::from_level(LZMA2_LEVEL),
    }
}

pub(super) fn write_archive(
    entries: &[PendingEntry],
    dest: &Path,
    archive_name: &str,
    total: usize,
    total_bytes: u64,
    progress: &SharedCompressionProgress,
    cancel: &CompressionCancelFlag,
) -> io::Result<()> {
    let mut writer = sevenz_rust2::ArchiveWriter::create(dest).map_err(seven_zip_error)?;
    writer.set_content_methods(vec![sevenz_rust2::EncoderConfiguration::from(
        lzma2_configuration(total_bytes, total),
    )]);

    let mut processed = 0usize;
    let mut bytes_done = 0u64;
    for entry in entries {
        if is_cancelled(cancel) {
            return Err(cancelled_error());
        }

        let archive_entry =
            sevenz_rust2::ArchiveEntry::from_path(&entry.source, entry.name.clone());
        if entry.is_dir {
            writer
                .push_archive_entry(archive_entry, None::<File>)
                .map_err(seven_zip_error)?;
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
        let reader = ProgressReader::new(
            File::open(&entry.source)?,
            processed,
            bytes_done,
            progress.clone(),
            cancel.clone(),
        );

        if let Err(error) = writer.push_archive_entry(archive_entry, Some(reader)) {
            // The crate wraps reader errors; recover the cancellation kind so
            // the worker reports "cancelled" instead of "failed".
            if is_cancelled(cancel) {
                return Err(cancelled_error());
            }
            return Err(seven_zip_error(error));
        }
        // Cancellation that landed while the pool drained an already-fed
        // entry (reader at EOF) must still abort the job.
        if is_cancelled(cancel) {
            return Err(cancelled_error());
        }

        processed += 1;
        bytes_done += entry.size;
    }

    if is_cancelled(cancel) {
        return Err(cancelled_error());
    }
    writer
        .finish()
        .map_err(|e| io::Error::other(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{mt_threads_for, MT_MAX_TOTAL_BYTES, MT_MIN_AVG_ENTRY_BYTES};

    const MANY_MIB: u64 = 64 << 20;

    #[test]
    fn mt_requires_at_least_four_cores() {
        assert_eq!(mt_threads_for(1, MANY_MIB, 1), None);
        assert_eq!(mt_threads_for(2, MANY_MIB, 1), None);
        assert_eq!(mt_threads_for(3, MANY_MIB, 1), None);
    }

    #[test]
    fn mt_thread_count_scales_with_cores_and_is_capped() {
        assert_eq!(mt_threads_for(4, MANY_MIB, 1), Some(2));
        assert_eq!(mt_threads_for(8, MANY_MIB, 1), Some(4));
        assert_eq!(mt_threads_for(16, MANY_MIB, 1), Some(8));
        assert_eq!(mt_threads_for(32, MANY_MIB, 1), Some(8));
    }

    #[test]
    fn mt_needs_large_average_entries() {
        // 1000 tiny files: per-entry pools would only add overhead.
        assert_eq!(mt_threads_for(8, 10 << 20, 1000), None);
        // Average entry exactly at the threshold enables MT.
        assert_eq!(
            mt_threads_for(8, MT_MIN_AVG_ENTRY_BYTES * 100, 100),
            Some(4)
        );
        // One huge file among many small ones still qualifies.
        assert_eq!(mt_threads_for(8, 560 << 20, 1), Some(4));
    }

    #[test]
    fn mt_is_disabled_above_the_size_gate() {
        assert_eq!(mt_threads_for(8, MT_MAX_TOTAL_BYTES, 1), Some(4));
        assert_eq!(mt_threads_for(8, MT_MAX_TOTAL_BYTES + 1, 1), None);
    }

    #[test]
    fn mt_requires_at_least_one_file() {
        assert_eq!(mt_threads_for(8, MANY_MIB, 0), None);
    }
}
