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
    assert_eq!(locations[0].kind, NetworkLocationKind::LinkFile);
    assert_eq!(locations[1].label, "Network Folder");
    assert_eq!(locations[1].kind, NetworkLocationKind::FolderShortcut);
    assert_eq!(locations[1].path, r"\\server\share\folder");
    let entry = locations[1].to_file_entry();
    assert_eq!(entry.path, temp.path().join("Network Folder"));
    assert!(locations[1].matches_shortcut_path(&entry.path));
    assert_eq!(entry.name, "Network Folder");
    assert!(entry.is_dir);
    assert!(entry.drive_info.is_none());
}

#[test]
fn network_locations_preserve_distinct_shortcuts_to_the_same_target_and_refresh() {
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
    assert_eq!(locations.len(), 2);
    assert_eq!(locations[0].label, "First");
    // A shortcut that cannot be loaded no longer contributes an entry.
    assert!(read_network_locations(temp.path(), |_| None).is_empty());
    assert!(read_network_locations(&temp.path().join("missing"), |_| unreachable!()).is_empty());
}

#[test]
fn saved_network_location_activation_resolves_the_target_but_not_arbitrary_folders() {
    let root = Path::new(r"C:\Network Shortcuts");
    let saved = root.join("Network Folder");
    let target = PathBuf::from(r"\\server\share\folder");
    assert_eq!(
        resolve_saved_location(root, &saved, |link| {
            assert_eq!(link, saved.join("target.lnk"));
            Some(target.clone())
        }),
        Some(target)
    );
    assert!(
        resolve_saved_location(root, Path::new(r"C:\Other Folder"), |_| panic!(
            "unrelated folder must not be resolved"
        ))
        .is_none()
    );
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
fn resolution_distinguishes_a_folder_named_lnk_from_a_direct_link_file() {
    let root = Path::new(r"C:\Network Shortcuts");
    let folder = root.join("Folder.lnk");
    let target = PathBuf::from(r"\\server\share");
    assert_eq!(
        resolve_saved_location(root, &folder, |path| {
            assert_eq!(path, folder.join("target.lnk"));
            Some(target.clone())
        }),
        Some(target.clone())
    );
    let link = root.join("Direct.lnk");
    assert_eq!(
        resolve_saved_location(root, &link, |path| {
            (path == link).then(|| target.clone())
        }),
        Some(target)
    );
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
    assert!(locations.iter().all(|location| {
        let source = Path::new(&location.shortcut_path);
        location.to_file_entry().path == source
            && resolve_network_location_target(source).is_some_and(|target| {
                network_location_path_key(&target)
                    == network_location_path_key(Path::new(&location.path))
            })
    }));
}
