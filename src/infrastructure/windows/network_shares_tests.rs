use super::*;
use windows::Win32::Storage::FileSystem::{STYPE_IPC, STYPE_PRINTQ, STYPE_TEMPORARY};

#[test]
fn only_disk_shares_become_navigable_entries() {
    let entry = share_entry("server", "share".into(), STYPE_DISKTREE.0, false).unwrap();
    assert_eq!(entry.path, PathBuf::from(r"\\server\share"));
    assert_eq!(entry.name, "share");
    assert!(entry.is_dir);
    assert!(!entry.is_hidden);
    assert!(entry.created.is_none());
    assert!(share_entry("server", "printer".into(), STYPE_PRINTQ.0, true).is_none());
    assert!(share_entry("server", "IPC$".into(), STYPE_IPC.0 | STYPE_SPECIAL.0, true).is_none());
    assert!(share_entry("server", "share".into(), STYPE_TEMPORARY.0, false).is_some());
}

#[test]
fn hidden_shares_follow_the_hidden_files_setting() {
    for (name, flags) in [("share$", 0), ("share", STYPE_SPECIAL.0)] {
        assert!(share_entry("server", name.into(), flags, false).is_none());
        assert!(
            share_entry("server", name.into(), flags, true)
                .unwrap()
                .is_hidden
        );
    }
}

#[test]
fn share_names_cannot_escape_the_server_container() {
    for name in ["", ".", "..", r"share\folder", "share/folder", "share\0"] {
        assert!(share_entry("server", name.into(), STYPE_DISKTREE.0, true).is_none());
    }
}

#[test]
fn non_server_paths_are_rejected_before_network_io() {
    assert_eq!(
        list_network_shares(r"\\server\share", false).unwrap_err(),
        ERROR_INVALID_PARAMETER.0
    );
}

#[test]
#[ignore = "Requires the Windows SMB server service and an existing visible disk share"]
fn enumerates_existing_loopback_shares_without_changing_them() {
    let shares = list_network_shares(r"\\localhost", false).unwrap();
    assert!(
        !shares.is_empty(),
        "Configure a visible disk share before running this test"
    );
    for share in shares {
        assert!(share.is_dir);
        assert!(!share.is_hidden);
        assert!(share.path.to_string_lossy().starts_with(r"\\localhost\"));
    }
}
