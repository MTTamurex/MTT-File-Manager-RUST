use super::*;
use crate::infrastructure::windows::DriveType;

fn info(drive_type: DriveType, file_system: &str) -> DriveInfo {
    DriveInfo {
        file_system: file_system.to_owned(),
        drive_type,
        total_space: 0,
        free_space: 0,
        model: None,
        serial_number: None,
        firmware_revision: None,
        bus_type: None,
        health: None,
    }
}

#[test]
fn current_path_selects_its_own_drive_root_without_using_selection() {
    for (path, expected) in [
        (r"C:\", 'C'),
        (r"d:\Folder\Nested", 'D'),
        ("d:/Folder/Nested", 'D'),
        (r"\\?\D:\Folder\Nested", 'D'),
    ] {
        let target = drive_for_path(Path::new(path), |root| {
            assert_eq!(root, format!("{expected}:\\"));
            Some(info(DriveType::Fixed, "NTFS"))
        });
        assert_eq!(target, Some(expected));
    }
}

#[test]
fn network_virtual_relative_and_device_paths_never_query_volume_metadata() {
    for path in [
        r"\\server\share\Folder",
        r"\\?\UNC\server\share\Folder",
        r"\\server",
        "::computer::",
        "::recycle_bin::",
        "::tag::1",
        "shell:NetworkPlacesFolder",
        "",
        "Folder",
        r"C:Folder",
        "C:",
        r"\\.\C:\Folder",
    ] {
        assert_eq!(
            drive_for_path(Path::new(path), |_| panic!(
                "must not query metadata for an unsupported location"
            )),
            None,
            "{path}"
        );
    }
}

#[test]
fn mapped_remote_unknown_and_optical_drives_remain_blocked_even_with_ntfs_metadata() {
    for drive_type in [DriveType::Remote, DriveType::Unknown, DriveType::Cdrom] {
        assert_eq!(
            drive_for_path(Path::new(r"Z:\Folder"), |_| Some(info(drive_type, "NTFS"))),
            None
        );
    }
}

#[test]
fn local_drives_require_ntfs_and_known_volume_information() {
    for drive_type in [DriveType::Fixed, DriveType::Removable, DriveType::RamDisk] {
        for file_system in ["NTFS", "ntfs"] {
            assert_eq!(
                drive_for_path(Path::new(r"C:\Folder"), |_| Some(info(
                    drive_type,
                    file_system
                ))),
                Some('C')
            );
        }
        for file_system in ["exFAT", "FAT32", "ReFS", ""] {
            assert_eq!(
                drive_for_path(Path::new(r"C:\Folder"), |_| Some(info(
                    drive_type,
                    file_system
                ))),
                None
            );
        }
    }
    assert_eq!(drive_for_path(Path::new(r"C:\Folder"), |_| None), None);
}
