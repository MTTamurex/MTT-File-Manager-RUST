use std::collections::HashSet;
use std::time::Duration;

use super::CandidateVolume;

pub(super) fn discover_candidate_volumes(
    service_volumes: &HashSet<char>,
    service_online: bool,
) -> Vec<CandidateVolume> {
    let mut candidates = Vec::new();
    let drives = crate::infrastructure::windows::get_all_drives();

    for (path, label) in drives {
        let Some(letter) = parse_drive_letter(&path) else {
            continue;
        };

        let volume = crate::infrastructure::windows::get_volume_info(&path);
        let file_system = volume.file_system;

        if should_index_volume(
            letter,
            &label,
            &file_system,
            service_volumes,
            service_online,
        ) {
            candidates.push(CandidateVolume {
                drive_letter: letter,
                label,
                file_system,
            });
        }
    }

    candidates
}

fn should_index_volume(
    drive_letter: char,
    label: &str,
    file_system: &str,
    service_volumes: &HashSet<char>,
    service_online: bool,
) -> bool {
    let missing_from_service = !service_volumes.contains(&drive_letter);
    if !missing_from_service {
        return false;
    }

    let virtual_indicator = is_virtual_indicator(label, file_system);
    if service_online {
        // The elevated service owns native local filesystems even when its
        // asynchronous startup discovery has not published this drive yet.
        // Session scans are reserved for user-session virtual mounts that the
        // service cannot access; duplicating FAT/exFAT scans can keep USB media
        // busy while Windows is trying to eject it.
        return virtual_indicator;
    }

    virtual_indicator
}

fn is_virtual_indicator(label: &str, file_system: &str) -> bool {
    let label_lower = label.to_ascii_lowercase();
    let fs_lower = file_system.to_ascii_lowercase();

    label_lower.contains("cryptomator")
        || fs_lower.contains("cryptofs")
        || fs_lower.contains("dokan")
        || fs_lower.contains("winfsp")
        || fs_lower == "fuse"
}

fn is_fat_family_fs(file_system: &str) -> bool {
    matches!(
        file_system.trim().to_ascii_lowercase().as_str(),
        "exfat" | "fat32" | "fat" | "fat16" | "fat12"
    )
}

pub(super) fn rescan_interval_for_volume(file_system: &str, label: &str) -> Duration {
    if is_virtual_indicator(label, file_system) {
        Duration::from_secs(30)
    } else if is_fat_family_fs(file_system) {
        Duration::from_secs(120)
    } else {
        Duration::from_secs(180)
    }
}

fn parse_drive_letter(path: &str) -> Option<char> {
    path.chars()
        .next()
        .map(|c| c.to_ascii_uppercase())
        .filter(|c| c.is_ascii_alphabetic())
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::should_index_volume;

    #[test]
    fn session_index_does_not_duplicate_service_fat32_scan() {
        assert!(!should_index_volume(
            'R',
            "",
            "FAT32",
            &HashSet::new(),
            true
        ));
        assert!(!should_index_volume(
            'R',
            "",
            "exFAT",
            &HashSet::new(),
            true
        ));
    }

    #[test]
    fn session_index_still_covers_user_session_virtual_mounts() {
        assert!(should_index_volume(
            'R',
            "",
            "WinFsp",
            &HashSet::new(),
            true
        ));
        assert!(should_index_volume(
            'R',
            "",
            "WinFsp",
            &HashSet::new(),
            false
        ));
    }

    #[test]
    fn service_covered_volumes_are_never_session_scanned() {
        let service_volumes = HashSet::from(['R']);
        assert!(!should_index_volume(
            'R',
            "",
            "FAT32",
            &service_volumes,
            true
        ));
        assert!(!should_index_volume(
            'R',
            "",
            "WinFsp",
            &service_volumes,
            true
        ));
    }
}
