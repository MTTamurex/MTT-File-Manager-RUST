use mtt_search_protocol::{SearchResponse, DRIVE_INDEXING_DISABLED_ERROR};
use std::path::{Component, Path, Prefix};
use windows::Win32::Foundation::HANDLE;

use crate::ipc_authorization::ClientIdentityError;
use crate::volume_lifecycle::VolumeLifecycle;

use super::pipe_io::send_response;

/// Any interactive user of the trusted app may edit machine-wide preferences.
/// Both rejected AND unavailable identities fail closed; no path-ACL fallback.
pub(super) fn handle(
    pipe: HANDLE,
    lifecycle: &VolumeLifecycle,
    update: Option<(char, bool)>,
    verify: impl FnOnce(HANDLE) -> Result<(), ClientIdentityError>,
) {
    if let Err(error) = verify(pipe) {
        eprintln!(
            "[IPC] Drive indexing configuration rejected: {}",
            crate::redact_paths(&error.to_string())
        );
        let _ = send_response(pipe, &SearchResponse::Error("Authorization failed".into()));
        return;
    }
    let response = match update {
        None => lifecycle.drive_indexing_config_response(),
        Some((letter, enabled)) => {
            let letter = letter.to_ascii_uppercase();
            match lifecycle.set_drive_indexing(letter, enabled) {
                Ok(()) => SearchResponse::DriveIndexingUpdated {
                    drive_letter: letter,
                    enabled,
                },
                Err(error) => {
                    eprintln!(
                        "[IPC] Drive indexing update failed: {}",
                        crate::redact_paths(&error)
                    );
                    SearchResponse::Error("Drive configuration update failed".into())
                }
            }
        }
    };
    let _ = send_response(pipe, &response);
}

pub(super) fn reject_disabled(pipe: HANDLE, lifecycle: &VolumeLifecycle, letter: char) -> bool {
    if lifecycle.is_indexing_enabled(letter) {
        return false;
    }
    let _ = send_response(
        pipe,
        &SearchResponse::Error(DRIVE_INDEXING_DISABLED_ERROR.into()),
    );
    true
}

pub(super) fn path_drive_letter(path: &str) -> Option<char> {
    let Component::Prefix(prefix) = Path::new(path).components().next()? else {
        return None;
    };
    match prefix.kind() {
        Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
            Some((letter as char).to_ascii_uppercase())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::path_drive_letter;

    #[test]
    fn policy_recognizes_standard_and_verbatim_drive_prefixes_only() {
        for path in [r"r:\folder", "R:/folder", r"\\?\r:\folder", "R:folder"] {
            assert_eq!(path_drive_letter(path), Some('R'));
        }
        for path in ["relative", r"\folder", r"\\server\share\folder"] {
            assert_eq!(path_drive_letter(path), None);
        }
    }
}
