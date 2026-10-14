//! Google Drive's own Explorer overlays, queried only on the sync-status worker.

use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::core::{Interface, GUID, HRESULT, PCWSTR, PWSTR};
use windows::Win32::Foundation::{ERROR_SUCCESS, S_FALSE, S_OK};
use windows::Win32::Storage::FileSystem::{GetFileAttributesW, INVALID_FILE_ATTRIBUTES};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER,
    HKEY_LOCAL_MACHINE, KEY_READ, RRF_RT_REG_SZ,
};
use windows::Win32::UI::Shell::IShellIconOverlayIdentifier;

use crate::domain::file_entry::SyncStatus;

const OVERLAY_KEY: &str =
    r"SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\ShellIconOverlayIdentifiers";

struct RegKey(HKEY);

impl Drop for RegKey {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

pub(super) struct Overlay {
    identifier: IShellIconOverlayIdentifier,
    status: SyncStatus,
    priority: i32,
}

fn overlay_status(name: &str) -> Option<SyncStatus> {
    match name.trim() {
        "GoogleDriveCloudOverlayIconHandler" => Some(SyncStatus::CloudOnly),
        "GoogleDrivePinnedOverlayIconHandler" => Some(SyncStatus::Pinned),
        "GoogleDriveProgressOverlayIconHandler" => Some(SyncStatus::Syncing),
        // Excluded mirror files must not receive a successful-sync badge.
        "GoogleDriveMirrorBlacklistedOverlayIconHandler" => Some(SyncStatus::None),
        _ => None,
    }
}

pub(super) fn load_overlays() -> Vec<Overlay> {
    let mut overlays = Vec::new();
    let mut seen = Vec::new();
    let key_wide: Vec<u16> = OVERLAY_KEY.encode_utf16().chain(Some(0)).collect();
    // Per-user registrations take precedence over machine-wide registrations.
    for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let mut key = HKEY::default();
        if unsafe { RegOpenKeyExW(root, PCWSTR(key_wide.as_ptr()), Some(0), KEY_READ, &mut key) }
            .is_err()
        {
            continue;
        }
        let key = RegKey(key);
        for index in 0.. {
            let mut name = [0u16; 512];
            let mut len = name.len() as u32;
            let result = unsafe {
                RegEnumKeyExW(
                    key.0,
                    index,
                    Some(PWSTR(name.as_mut_ptr())),
                    &mut len,
                    None,
                    None,
                    None,
                    None,
                )
            };
            if result != ERROR_SUCCESS {
                break;
            }
            let name_text = String::from_utf16_lossy(&name[..len as usize]);
            let Some(status) = overlay_status(&name_text) else {
                continue;
            };
            if seen.contains(&name_text.trim().to_string()) {
                continue;
            }
            seen.push(name_text.trim().to_string());
            let mut value = [0u16; 64];
            let mut bytes = std::mem::size_of_val(&value) as u32;
            if unsafe {
                RegGetValueW(
                    key.0,
                    PCWSTR(name.as_ptr()),
                    PCWSTR::null(),
                    RRF_RT_REG_SZ,
                    None,
                    Some(value.as_mut_ptr().cast()),
                    Some(&mut bytes),
                )
            }
            .is_err()
            {
                continue;
            }
            let end = value.iter().position(|&c| c == 0).unwrap_or(value.len());
            let clsid_text = String::from_utf16_lossy(&value[..end]);
            let Ok(clsid) = GUID::try_from(clsid_text.trim().trim_matches(['{', '}'])) else {
                continue;
            };
            let identifier: windows::core::Result<IShellIconOverlayIdentifier> =
                unsafe { CoCreateInstance(&clsid, None, CLSCTX_INPROC_SERVER) };
            let Ok(identifier) = identifier else { continue };
            // Explorer initializes overlays through GetOverlayInfo before IsMemberOf.
            let mut icon_file = [0u16; 32768];
            let mut icon_index = 0;
            let mut flags = 0;
            if unsafe { identifier.GetOverlayInfo(&mut icon_file, &mut icon_index, &mut flags) }
                .is_err()
            {
                continue;
            }
            let priority = unsafe { identifier.GetPriority() }.unwrap_or(100);
            overlays.push(Overlay {
                identifier,
                status,
                priority,
            });
        }
    }
    overlays.sort_by_key(|overlay| overlay.priority);
    overlays
}

fn select_status(results: impl IntoIterator<Item = (SyncStatus, HRESULT)>) -> Option<SyncStatus> {
    let mut failed = false;
    for (status, result) in results {
        if result == S_OK {
            return Some(status);
        }
        if result != S_FALSE {
            failed = true;
        }
    }
    // No overlay means no badge, not an inferred successful/local status.
    if failed {
        None
    } else {
        Some(SyncStatus::None)
    }
}

pub(super) fn query_status(overlays: &[Overlay], path: &Path) -> Option<SyncStatus> {
    if overlays.is_empty() {
        return None;
    }
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let attrs = unsafe { GetFileAttributesW(PCWSTR(wide.as_ptr())) };
    if attrs == INVALID_FILE_ATTRIBUTES {
        return None;
    }
    select_status(overlays.iter().map(|overlay| {
        // The generated Result<()> wrapper discards S_FALSE. Preserve the raw
        // HRESULT: only S_OK means membership, despite both being success codes.
        let result = unsafe {
            (overlay.identifier.vtable().IsMemberOf)(
                overlay.identifier.as_raw(),
                PCWSTR(wide.as_ptr()),
                attrs,
            )
        };
        (overlay.status, result)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn false_membership_does_not_claim_cloud_or_pinned_status() {
        assert_eq!(
            select_status([
                (SyncStatus::CloudOnly, S_FALSE),
                (SyncStatus::Pinned, S_FALSE)
            ]),
            Some(SyncStatus::None)
        );
    }

    #[test]
    fn first_matching_overlay_wins_in_priority_order() {
        assert_eq!(
            select_status([
                (SyncStatus::Syncing, S_FALSE),
                (SyncStatus::CloudOnly, S_OK),
                (SyncStatus::Pinned, S_OK)
            ]),
            Some(SyncStatus::CloudOnly)
        );
        assert_eq!(
            select_status([(SyncStatus::Syncing, S_OK), (SyncStatus::Pinned, S_OK)]),
            Some(SyncStatus::Syncing)
        );
    }

    #[test]
    fn failed_queries_and_excluded_mirror_files_are_not_successful_sync() {
        assert_eq!(
            select_status([(SyncStatus::Pinned, HRESULT(0x80004005u32 as i32))]),
            None
        );
        assert_eq!(
            select_status([(SyncStatus::None, S_OK)]),
            Some(SyncStatus::None)
        );
        assert_eq!(
            overlay_status("    GoogleDrivePinnedOverlayIconHandler"),
            Some(SyncStatus::Pinned)
        );
        assert_eq!(overlay_status("OtherProviderOverlay"), None);
    }

    #[test]
    #[ignore = "requires Google Drive for Desktop shell extensions installed"]
    fn installed_overlays_can_be_queried_without_reading_file_contents() {
        let com = crate::infrastructure::windows::ComScope::sta();
        assert!(com.is_initialized());
        let overlays = load_overlays();
        assert!(overlays.iter().any(|o| o.status == SyncStatus::CloudOnly));
        assert!(overlays.iter().any(|o| o.status == SyncStatus::Pinned));
        assert!(overlays.iter().any(|o| o.status == SyncStatus::Syncing));
        let directory = tempfile::tempdir().unwrap();
        assert_eq!(
            query_status(&overlays, directory.path()),
            Some(SyncStatus::None)
        );
    }
}
