//! Remembers the icon file behind each cloud-root icon so a periodic refresh
//! skips the Shell extraction when that file did not change.
//!
//! Google Drive can rewrite its shortcut/icon in place, so every drive scan
//! schedules a refresh. Extracting through the Shell loads shell, COM and the
//! provider's Shell extension into the process; doing that every scan of an
//! idle window keeps tens of MB of working set busy for an identical image.
//! The file's size and modification time are read on the icon worker thread,
//! never on the UI thread, because the file lives on a cloud virtual drive.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileStamp {
    len: u64,
    modified: Option<SystemTime>,
}

/// Identity of the files that determine a cloud-root icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct IconFileStamp {
    resource: Option<FileStamp>,
    /// `desktop.ini` inside a folder resource, which names the folder icon.
    desktop_ini: Option<FileStamp>,
}

fn file_stamp(path: &Path) -> Option<FileStamp> {
    let metadata = std::fs::metadata(path).ok()?;
    Some(FileStamp {
        len: metadata.len(),
        modified: metadata.modified().ok(),
    })
}

/// Reads the stamp of an icon resource path. Returns `None` when nothing could
/// be read, which never counts as unchanged.
pub(super) fn icon_file_stamp(resource_path: Option<&Path>) -> Option<IconFileStamp> {
    let path = resource_path?;
    let resource = file_stamp(path);
    let desktop_ini = path
        .is_dir()
        .then(|| file_stamp(&path.join("desktop.ini")))
        .flatten();
    (resource.is_some() || desktop_ini.is_some()).then_some(IconFileStamp {
        resource,
        desktop_ini,
    })
}

/// Shared between the UI thread (clears) and icon workers (compare/record).
#[derive(Clone, Default)]
pub(super) struct CloudIconStamps(Arc<Mutex<HashMap<String, IconFileStamp>>>);

impl CloudIconStamps {
    /// Records `stamp` for `key` and reports whether it matches the stamp of
    /// the last extraction. A missing stamp is never unchanged.
    pub(super) fn record_and_check_unchanged(
        &self,
        key: &str,
        stamp: Option<IconFileStamp>,
    ) -> bool {
        let mut stamps = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match stamp {
            Some(stamp) => stamps.insert(key.to_string(), stamp) == Some(stamp),
            None => {
                stamps.remove(key);
                false
            }
        }
    }

    pub(super) fn clear(&self) {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unchanged_file_is_detected_after_the_first_extraction() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("icon.lnk");
        std::fs::write(&file, b"one").unwrap();
        let stamps = CloudIconStamps::default();

        assert!(!stamps.record_and_check_unchanged("key", icon_file_stamp(Some(&file))));
        assert!(stamps.record_and_check_unchanged("key", icon_file_stamp(Some(&file))));

        std::fs::write(&file, b"changed").unwrap();
        assert!(!stamps.record_and_check_unchanged("key", icon_file_stamp(Some(&file))));
    }

    #[test]
    fn unreadable_resource_always_extracts() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing.lnk");
        let stamps = CloudIconStamps::default();

        assert!(icon_file_stamp(Some(&missing)).is_none());
        assert!(!stamps.record_and_check_unchanged("key", None));
        assert!(!stamps.record_and_check_unchanged("key", None));
    }

    #[test]
    fn folder_resource_tracks_its_desktop_ini() {
        let dir = tempfile::tempdir().unwrap();
        let ini = dir.path().join("desktop.ini");
        std::fs::write(&ini, b"[.ShellClassInfo]").unwrap();
        let stamps = CloudIconStamps::default();

        assert!(!stamps.record_and_check_unchanged("key", icon_file_stamp(Some(dir.path()))));
        std::fs::write(&ini, b"[.ShellClassInfo]\nIconResource=icon.ico,0").unwrap();
        assert!(!stamps.record_and_check_unchanged("key", icon_file_stamp(Some(dir.path()))));
    }
}
