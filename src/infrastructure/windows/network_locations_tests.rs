use super::*;

#[test]
fn saved_network_locations_resolve_local_shortcuts_without_probing_remote_targets() {
    let temp = tempfile::tempdir().unwrap();
    for name in ["Network Folder", "Broken", "Local Folder"] {
        std::fs::create_dir(temp.path().join(name)).unwrap();
    }
    std::fs::write(temp.path().join("Direct.lnk"), b"shortcut").unwrap();
    std::fs::write(temp.path().join("desktop.ini"), b"ignored").unwrap();
    let locations = read_network_locations(temp.path(), |shortcut| {
        if shortcut == temp.path().join("Network Folder").join("target.lnk") {
            Some(PathBuf::from(r"\\server\share\folder"))
        } else if shortcut == temp.path().join("Direct.lnk") {
            Some(PathBuf::from(r"\\server\other"))
        } else if shortcut == temp.path().join("Local Folder").join("target.lnk") {
            Some(PathBuf::from(r"C:\Folder"))
        } else {
            None
        }
    });
    assert_eq!(locations.len(), 2);
    assert_eq!(locations[0].label, "Direct");
    assert_eq!(locations[1].label, "Network Folder");
    assert_eq!(locations[1].path, r"\\server\share\folder");
    let entry = locations[1].to_file_entry();
    assert_eq!(entry.path, PathBuf::from(r"\\server\share\folder"));
    assert_eq!(entry.name, "Network Folder");
    assert!(entry.is_dir);
    assert!(entry.drive_info.is_none());
    assert!(locations[1].matches_path(Path::new(r"\\SERVER\SHARE\folder\")));
}

#[test]
fn network_locations_deduplicate_equivalent_targets_and_refresh_from_current_shortcuts() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("First.lnk"), b"shortcut").unwrap();
    std::fs::write(temp.path().join("Second.lnk"), b"shortcut").unwrap();
    let locations = read_network_locations(temp.path(), |shortcut| {
        Some(PathBuf::from(if shortcut.file_stem().unwrap() == "First" {
            r"\\server\share"
        } else {
            r"\\SERVER\SHARE\"
        }))
    });
    assert_eq!(locations.len(), 1);
    assert_eq!(locations[0].label, "First");
    // A shortcut that cannot be loaded no longer contributes an entry.
    assert!(read_network_locations(temp.path(), |_| None).is_empty());
    assert!(read_network_locations(&temp.path().join("missing"), |_| unreachable!()).is_empty());
}

#[test]
fn network_locations_accept_filesystem_unc_targets_only() {
    for target in [
        r"\\server",
        r"\\server\share",
        r"\\?\UNC\server\share\folder",
    ] {
        assert!(is_network_target(Path::new(target)), "{target}");
    }
    for target in [
        "",
        r"C:\Folder",
        r"\\?\C:\Folder",
        r"\\.\pipe\name",
        "https://example.com",
        "ftp://example.com",
    ] {
        assert!(!is_network_target(Path::new(target)), "{target}");
    }
}

#[test]
#[ignore = "Requires a filesystem network location already saved in Windows"]
fn network_locations_enumerate_existing_windows_shortcuts_read_only() {
    let locations = get_network_locations();
    assert!(
        !locations.is_empty(),
        "Save a network location in Windows before running this test"
    );
    assert!(locations
        .iter()
        .all(|location| is_network_target(Path::new(&location.path))));
}
