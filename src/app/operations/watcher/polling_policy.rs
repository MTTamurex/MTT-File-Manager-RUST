//! FAT-family folders use the existing consistency probes instead of keeping
//! directory notification handles open in the interactive file manager.
use std::collections::HashMap;
use std::path::PathBuf;

pub(super) fn uses_polling(file_system: &str) -> bool {
    matches!(
        file_system.trim().to_ascii_uppercase().as_str(),
        "FAT" | "FAT12" | "FAT16" | "FAT32" | "EXFAT"
    )
}

/// Called on the setup worker, so filesystem discovery never blocks the UI.
/// Filter every path independently: the active/inactive panels may use
/// different filesystems, and Miller adds several ancestors on the same drive.
pub(super) fn native_watch_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    native_watch_paths_with(paths, |path| {
        crate::infrastructure::windows::get_file_system_for_path(path)
    })
}

fn native_watch_paths_with(
    paths: Vec<PathBuf>,
    mut filesystem_for_path: impl FnMut(&std::path::Path) -> Option<String>,
) -> Vec<PathBuf> {
    let mut filesystems = HashMap::<char, Option<String>>::new();
    paths
        .into_iter()
        .filter(|path| {
            let Some(letter) = crate::infrastructure::windows::extract_drive_letter(path) else {
                return true;
            };
            if crate::infrastructure::removal_aware_watcher::is_suspended(letter) {
                return false;
            }
            let filesystem = filesystems
                .entry(letter)
                .or_insert_with(|| filesystem_for_path(path));
            !filesystem.as_deref().is_some_and(uses_polling)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{native_watch_paths_with, uses_polling};
    use std::path::PathBuf;

    #[test]
    fn all_fat_variants_use_handle_free_monitoring() {
        for filesystem in ["FAT", "FAT12", "FAT16", "FAT32", "exFAT", " fat32 "] {
            assert!(uses_polling(filesystem));
        }
    }

    #[test]
    fn journal_and_virtual_filesystems_keep_native_monitoring() {
        for filesystem in ["NTFS", "ReFS", "WinFsp", "Dokan", "FUSE", ""] {
            assert!(!uses_polling(filesystem));
        }
    }

    #[test]
    fn mixed_panels_and_miller_ancestors_filter_each_drive_once() {
        let mut queried = Vec::new();
        let paths = vec![
            PathBuf::from(r"R:\Folder\Child"),
            PathBuf::from(r"R:\Folder"),
            PathBuf::from(r"R:\"),
            PathBuf::from(r"S:\Folder"),
            PathBuf::from(r"T:\Folder"),
        ];
        let native = native_watch_paths_with(paths, |path| {
            let letter = crate::infrastructure::windows::extract_drive_letter(path).unwrap();
            queried.push(letter);
            Some(
                match letter {
                    'R' => "FAT32",
                    'S' => "exFAT",
                    _ => "NTFS",
                }
                .to_string(),
            )
        });
        assert_eq!(native, vec![PathBuf::from(r"T:\Folder")]);
        assert_eq!(queried, vec!['R', 'S', 'T']);
    }
}
