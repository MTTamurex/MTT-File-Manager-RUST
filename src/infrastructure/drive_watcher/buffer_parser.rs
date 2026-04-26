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

            // M-21: bounds-check the filename slice before dereferencing kernel data.
            // Corrupted or truncated buffers can have FileNameLength pointing past the end.
            let buf_start = buffer.as_ptr() as usize;
            let name_start = (name_ptr as usize).wrapping_sub(buf_start);
            let name_end = name_start.saturating_add(name_len * 2);
            if name_end > buffer.len() {
                break; // corrupted entry — stop parsing
            }

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
            // M-21: guard against wrap-around from corrupt NextEntryOffset
            let next_offset = offset.saturating_add(info.NextEntryOffset as usize);
            if next_offset <= offset || next_offset > buffer.len() {
                break; // corrupt offset — stop parsing
            }
            offset = next_offset;
        }
    }

    // Defensive fallback: unmatched OLD rename at buffer end.
    if let Some(old_path) = pending_rename_old.take() {
        events.push(DriveWatcherEvent::Renamed(old_path.clone(), old_path));
    }

    events
}

fn path_matches_prefix(path: &Path, prefix: &Path) -> bool {
    // Zero-allocation case-insensitive path prefix comparison.
    // Windows paths are always valid Unicode (UTF-16) so `to_string_lossy` is lossless
    // for paths produced by ReadDirectoryChangesW.
    let path_raw = path.to_string_lossy();
    let prefix_raw = prefix.to_string_lossy();

    if prefix_raw.is_empty() {
        return true;
    }

    // Strip \\?\ prefix if present
    let path_str = path_raw.strip_prefix(r"\\?\").unwrap_or(&path_raw);
    let prefix_str = prefix_raw.strip_prefix(r"\\?\").unwrap_or(&prefix_raw);

    let path_bytes = path_str.as_bytes();
    let prefix_bytes = prefix_str.as_bytes();

    // Ensure we compare prefix with trailing backslash semantics.
    let prefix_has_trailing = prefix_bytes.last() == Some(&b'\\');
    let prefix_len_no_trail = if prefix_has_trailing {
        prefix_bytes.len() - 1
    } else {
        prefix_bytes.len()
    };

    // Match children: path starts with "prefix\"
    if path_bytes.len() > prefix_len_no_trail
        && path_bytes.get(prefix_len_no_trail) == Some(&b'\\')
        && path_bytes[..prefix_len_no_trail]
            .eq_ignore_ascii_case(&prefix_bytes[..prefix_len_no_trail])
    {
        return true;
    }

    // Match the prefix itself (e.g. path="c:\teste", prefix="c:\teste")
    // Critical for detecting when the watched folder itself is deleted or renamed.
    if path_bytes.len() == prefix_len_no_trail
        && path_bytes.eq_ignore_ascii_case(&prefix_bytes[..prefix_len_no_trail])
    {
        return true;
    }

    // Match ancestors: path is a parent of prefix (e.g. path="c:\parent", prefix="c:\parent\sub")
    // Needed when a parent folder of the watched path is deleted.
    let path_len = path_bytes.len();
    if prefix_len_no_trail > path_len
        && prefix_bytes.get(path_len) == Some(&b'\\')
        && prefix_bytes[..path_len].eq_ignore_ascii_case(path_bytes)
    {
        return true;
    }

    // Special case: if prefix is drive root (e.g., "d:\"), any path on that drive matches.
    prefix_len_no_trail <= 2
        && path_bytes.len() >= 2
        && path_bytes[0..1].eq_ignore_ascii_case(&prefix_bytes[0..1])
        && path_bytes[1] == b':'
}

/// Check if an event matches the current prefix.
pub(super) fn event_matches_prefix(event: &DriveWatcherEvent, prefix: &Path) -> bool {
    match event {
        DriveWatcherEvent::DriveLost(_) => true, // Always propagate.
        DriveWatcherEvent::PrefixInvalidated(p) => path_matches_prefix(p, prefix),
        DriveWatcherEvent::Created(p)
        | DriveWatcherEvent::Deleted(p)
        | DriveWatcherEvent::Modified(p)
        | DriveWatcherEvent::Unknown(p) => path_matches_prefix(p, prefix),
        DriveWatcherEvent::Renamed(old, new) => {
            path_matches_prefix(old, prefix) || path_matches_prefix(new, prefix)
        }
    }
}
