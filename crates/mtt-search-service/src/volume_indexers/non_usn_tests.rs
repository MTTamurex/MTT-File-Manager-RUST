use super::{is_fat_family_fs, max_non_usn_scan_interval, non_usn_scan_cadence};
use std::time::Duration;

#[test]
fn fat_family_keeps_a_bounded_polling_interval() {
    for file_system in ["FAT32", "exFAT", "FAT16"] {
        assert!(is_fat_family_fs(&file_system.to_ascii_lowercase()));
        let cadence = non_usn_scan_cadence(file_system);
        assert_eq!(cadence.periodic_interval, Duration::from_secs(120));
        assert_eq!(
            max_non_usn_scan_interval(&file_system.to_ascii_lowercase(), cadence.periodic_interval),
            cadence.periodic_interval
        );
    }
}

#[test]
fn non_fat_filesystems_keep_adaptive_idle_backoff() {
    let cadence = non_usn_scan_cadence("NTFS-like");
    assert_eq!(
        max_non_usn_scan_interval("ntfs-like", cadence.periodic_interval),
        cadence.periodic_interval * 3
    );
}
