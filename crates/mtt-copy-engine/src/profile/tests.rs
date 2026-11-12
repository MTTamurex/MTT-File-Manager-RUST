use super::*;

fn ssd(disk: u32) -> Device {
    Device {
        disk: Some(disk),
        kind: Kind::Ssd,
    }
}

#[test]
fn same_physical_disk_including_different_partitions_has_one_large_transfer() {
    let policy = choose(&[ssd(1)], ssd(1), Some(0));
    assert_eq!(policy.info.workers, 2);
    assert_eq!(policy.info.large_workers, 1);
    assert!(policy.info.shared_device);
    assert_eq!(policy.resources, vec![Resource::Disk(1)]);
}

#[test]
fn windows_disk_is_protected_even_when_the_other_endpoint_is_separate() {
    let policy = choose(&[ssd(0)], ssd(1), Some(0));
    assert!(policy.info.system_device);
    assert_eq!(policy.info.large_workers, 1);
    assert_eq!(policy.info.workers, 2);
}

#[test]
fn independent_data_ssds_keep_bounded_parallelism() {
    let policy = choose(&[ssd(1)], ssd(2), Some(0));
    assert_eq!(policy.info.workers, 4);
    assert_eq!(policy.info.large_workers, 2);
    assert!(!policy.info.conservative);
}

#[test]
fn hdd_remote_unknown_or_missing_system_identity_uses_one_worker() {
    for device in [
        Device {
            disk: Some(1),
            kind: Kind::Hdd,
        },
        Device {
            disk: None,
            kind: Kind::Unknown,
        },
        Device {
            disk: None,
            kind: Kind::Remote,
        },
    ] {
        let policy = choose(&[device], ssd(2), Some(0));
        assert_eq!(policy.info.workers, 1);
        assert_eq!(policy.info.large_workers, 1);
    }
    assert_eq!(choose(&[ssd(1)], ssd(2), None).info.workers, 1);
    assert!(
        choose(&[ssd(1), ssd(2)], ssd(2), Some(0))
            .info
            .shared_device
    );
}

#[cfg(windows)]
#[test]
fn real_volume_probe_never_parallelizes_large_files_on_the_same_volume() {
    let directory = tempfile::tempdir().unwrap();
    let policy = profile(&[directory.path().to_path_buf()], directory.path());
    assert_eq!(policy.info.large_workers, 1);
    assert!(policy.info.workers <= 2);
    assert_eq!(policy.resources.len(), 1);
}
