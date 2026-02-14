use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use windows::Win32::Storage::FileSystem::{
    FILE_ACTION_ADDED, FILE_ACTION_MODIFIED, FILE_ACTION_REMOVED, FILE_ACTION_RENAMED_NEW_NAME,
    FILE_ACTION_RENAMED_OLD_NAME, FILE_NOTIFY_INFORMATION,
};

use super::DriveWatcherEvent;

/// Parse FILE_NOTIFY_INFORMATION buffer into events.
pub(super) fn parse_notify_buffer(buffer: &[u8], drive_root: &Path) -> Vec<DriveWatcherEvent> {
    let mut events = Vec::new();
    let mut offset = 0usize;
    let mut pending_rename_old: Option<PathBuf> = None;

    // Ensure drive_root ends with backslash for proper path construction.
    let drive_root_str = drive_root.to_string_lossy();
    let drive_root_normalized = if drive_root_str.ends_with('\\') {
        drive_root_str.to_string()
    } else {
        format!("{}\\", drive_root_str)
    };

    unsafe {
        loop {
            if offset + std::mem::size_of::<FILE_NOTIFY_INFORMATION>() > buffer.len() {
                break;
            }

            let info = &*(buffer.as_ptr().add(offset) as *const FILE_NOTIFY_INFORMATION);

            // Extract filename (comes as relative path from watched directory).
            let name_len = info.FileNameLength as usize / 2;
            let name_ptr = info.FileName.as_ptr();
            let name_slice = std::slice::from_raw_parts(name_ptr, name_len);
            let filename = OsString::from_wide(name_slice);
            let filename_str = filename.to_string_lossy();

            // Build full path manually to avoid Path::join edge cases.
            let full_path_str = format!("{}{}", drive_root_normalized, filename_str);
            let full_path = PathBuf::from(full_path_str);

            match info.Action {
                FILE_ACTION_ADDED => events.push(DriveWatcherEvent::Created(full_path)),
                FILE_ACTION_REMOVED => events.push(DriveWatcherEvent::Deleted(full_path)),
                FILE_ACTION_MODIFIED => events.push(DriveWatcherEvent::Modified(full_path)),
                FILE_ACTION_RENAMED_OLD_NAME => {
                    if let Some(unpaired_old) = pending_rename_old.replace(full_path) {
                        // Two OLD events in a row (rare): flush previous one conservatively.
                        events.push(DriveWatcherEvent::Renamed(
                            unpaired_old.clone(),
                            unpaired_old,
                        ));
                    }
                }
                FILE_ACTION_RENAMED_NEW_NAME => {
                    if let Some(old_path) = pending_rename_old.take() {
                        events.push(DriveWatcherEvent::Renamed(old_path, full_path));
                    } else {
                        // Defensive fallback for unmatched NEW event.
                        events.push(DriveWatcherEvent::Renamed(full_path.clone(), full_path));
                    }
                }
                _ => events.push(DriveWatcherEvent::Unknown(full_path)),
            }

            if info.NextEntryOffset == 0 {
                break;
            }
            offset += info.NextEntryOffset as usize;
        }
    }

    // Defensive fallback: unmatched OLD rename at buffer end.
    if let Some(old_path) = pending_rename_old.take() {
        events.push(DriveWatcherEvent::Renamed(old_path.clone(), old_path));
    }

    events
}

fn path_matches_prefix(path: &Path, prefix: &Path) -> bool {
    // Normalize both paths for comparison.
    let path_str_raw = path.to_string_lossy().to_lowercase();
    let prefix_str_raw = prefix.to_string_lossy().to_lowercase();

    if prefix_str_raw.is_empty() {
        return true;
    }

    let path_str = path_str_raw.strip_prefix(r"\\?\").unwrap_or(&path_str_raw);
    let prefix_str = prefix_str_raw.strip_prefix(r"\\?\").unwrap_or(&prefix_str_raw);

    // Ensure prefix ends with backslash for proper prefix matching.
    let prefix_normalized = if prefix_str.ends_with('\\') {
        prefix_str.to_string()
    } else {
        format!("{}\\", prefix_str)
    };

    path_str.starts_with(&prefix_normalized)
        // Special case: if prefix is drive root (e.g., "d:\\"), any path on that drive matches.
        || (prefix_normalized.len() == 3 && path_str.starts_with(&prefix_normalized[..2]))
}

/// Check if an event matches the current prefix.
pub(super) fn event_matches_prefix(event: &DriveWatcherEvent, prefix: &Path) -> bool {
    match event {
        DriveWatcherEvent::DriveLost(_) => true, // Always propagate.
        DriveWatcherEvent::Created(p)
        | DriveWatcherEvent::Deleted(p)
        | DriveWatcherEvent::Modified(p)
        | DriveWatcherEvent::Unknown(p) => path_matches_prefix(p, prefix),
        DriveWatcherEvent::Renamed(old, new) => {
            path_matches_prefix(old, prefix) || path_matches_prefix(new, prefix)
        }
    }
}
