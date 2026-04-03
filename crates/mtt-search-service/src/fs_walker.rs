use std::collections::VecDeque;
use std::os::windows::fs::MetadataExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::file_index::VolumeIndex;

/// Synthetic root reference used by `path_resolver` for non-USN volumes.
const ROOT_REF: u64 = 5;
/// First generated synthetic reference.
const FIRST_SYNTHETIC_REF: u64 = ROOT_REF + 1;
/// FILE_ATTRIBUTE_REPARSE_POINT
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;

#[derive(Debug, Clone, Copy)]
pub struct ScanStats {
    pub records_indexed: usize,
    pub directories_scanned: usize,
    pub errors: usize,
    pub elapsed: Duration,
}

/// Full-tree scan for filesystems without USN support (FAT/exFAT/FUSE/CryptoFS).
///
/// The scan is iterative (no recursion) and skips reparse points to avoid cycles.
pub fn scan_volume(
    drive_letter: char,
    index: &mut VolumeIndex,
    shutdown: &AtomicBool,
) -> Result<ScanStats, String> {
    let root = PathBuf::from(format!("{}:\\", drive_letter));
    if !root.exists() {
        return Err(format!("volume root {}:\\ is not accessible", drive_letter));
    }

    let start = Instant::now();
    let mut next_ref = FIRST_SYNTHETIC_REF;
    let mut queue = VecDeque::new();
    let mut directories_scanned = 0usize;
    let mut errors = 0usize;
    queue.push_back((root, ROOT_REF));

    while let Some((dir_path, parent_ref)) = queue.pop_front() {
        // Check shutdown every 100 directories to allow graceful stop.
        if directories_scanned.is_multiple_of(100) && shutdown.load(Ordering::Relaxed) {
            return Err("scan interrupted by shutdown".to_string());
        }
        directories_scanned += 1;

        let entries = match std::fs::read_dir(&dir_path) {
            Ok(entries) => entries,
            Err(_) => {
                errors += 1;
                continue;
            }
        };

        for entry_result in entries {
            let entry = match entry_result {
                Ok(entry) => entry,
                Err(_) => {
                    errors += 1;
                    continue;
                }
            };

            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => {
                    errors += 1;
                    continue;
                }
            };

            let entry_ref = next_ref;
            next_ref = next_ref
                .checked_add(1)
                .ok_or_else(|| "synthetic reference counter overflowed".to_string())?;

            let name = entry.file_name().to_string_lossy().into_owned();
            let is_dir = file_type.is_dir();
            if !index.insert_record(entry_ref, &name, parent_ref, is_dir) {
                eprintln!("[FS-WALKER] Name arena full — stopping scan");
                break;
            }

            if !is_dir || file_type.is_symlink() {
                continue;
            }

            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(_) => {
                    errors += 1;
                    continue;
                }
            };

            if (metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0 {
                continue;
            }

            queue.push_back((entry.path(), entry_ref));
        }
    }

    Ok(ScanStats {
        records_indexed: index.records.len(),
        directories_scanned,
        errors,
        elapsed: start.elapsed(),
    })
}
