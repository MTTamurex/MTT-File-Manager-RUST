//! Eligibility shared by disk analyzer entry points.

use crate::infrastructure::windows::{normalize_drive_root_path, DriveType};
use std::path::Path;

pub(crate) fn is_local_volume(drive_type: DriveType) -> bool {
    matches!(
        drive_type,
        DriveType::Fixed | DriveType::Removable | DriveType::RamDisk
    )
}

pub(crate) fn supports_disk_analysis(
    path: &Path,
    drive_type: DriveType,
    file_system: &str,
) -> bool {
    normalize_drive_root_path(path).is_some()
        && is_local_volume(drive_type)
        && file_system.eq_ignore_ascii_case("NTFS")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disk_analysis_rejects_network_drives_even_when_the_server_reports_ntfs() {
        let root = Path::new("Z:\\");
        for drive_type in [DriveType::Remote, DriveType::Unknown, DriveType::Cdrom] {
            assert!(!supports_disk_analysis(root, drive_type, "NTFS"));
        }
    }

    #[test]
    fn disk_analysis_remains_available_for_local_ntfs_volumes_only() {
        let root = Path::new("C:\\");
        for drive_type in [DriveType::Fixed, DriveType::Removable, DriveType::RamDisk] {
            assert!(supports_disk_analysis(root, drive_type, "NTFS"));
            assert!(supports_disk_analysis(root, drive_type, "ntfs"));
            assert!(!supports_disk_analysis(root, drive_type, "exFAT"));
            assert!(!supports_disk_analysis(root, drive_type, ""));
        }
    }

    #[test]
    fn disk_analysis_rejects_network_locations_and_non_root_paths_even_with_local_metadata() {
        for path in [
            r"\\server\share",
            r"\\server",
            r"C:\folder",
            "",
            "shell:NetworkPlacesFolder",
        ] {
            assert!(!supports_disk_analysis(
                Path::new(path),
                DriveType::Fixed,
                "NTFS"
            ));
        }
    }
}
