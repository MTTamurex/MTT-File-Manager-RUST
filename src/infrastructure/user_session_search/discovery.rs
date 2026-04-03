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
        return virtual_indicator || !is_usn_filesystem(file_system);
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

fn is_usn_filesystem(file_system: &str) -> bool {
    file_system.eq_ignore_ascii_case("NTFS") || file_system.eq_ignore_ascii_case("ReFS")
}

fn parse_drive_letter(path: &str) -> Option<char> {
    path.chars()
        .next()
        .map(|c| c.to_ascii_uppercase())
        .filter(|c| c.is_ascii_alphabetic())
}
