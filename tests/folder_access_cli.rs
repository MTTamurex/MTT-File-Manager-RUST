#![cfg(windows)]

use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::process::Command;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{LocalFree, HLOCAL};
use windows::Win32::Security::Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT};
use windows::Win32::Security::{
    GetSecurityDescriptorLength, DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
};

fn acl_snapshot(path: &Path) -> Vec<u8> {
    let path: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    unsafe {
        assert_eq!(
            GetNamedSecurityInfoW(
                PCWSTR(path.as_ptr()),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                None,
                None,
                &mut descriptor
            )
            .0,
            0
        );
        let len = GetSecurityDescriptorLength(descriptor) as usize;
        let bytes = std::slice::from_raw_parts(descriptor.0.cast::<u8>(), len).to_vec();
        let _ = LocalFree(Some(HLOCAL(descriptor.0)));
        bytes
    }
}

#[test]
fn executable_rejects_legacy_grants_without_modifying_acl() {
    let folder = tempfile::tempdir().unwrap();
    let before = acl_snapshot(folder.path());
    for sid in ["S-1-1-0", "S-1-5-32-544", "S-1-5-18"] {
        let status = Command::new(env!("CARGO_BIN_EXE_mtt-file-manager"))
            .arg("--grant-folder-access")
            .arg(folder.path())
            .arg(sid)
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(87));
        assert!(
            before == acl_snapshot(folder.path()),
            "legacy helper changed the ACL"
        );
    }
}

#[test]
fn executable_rejects_sid_appended_to_the_new_protocol() {
    let status = Command::new(env!("CARGO_BIN_EXE_mtt-file-manager"))
        .args([
            "--grant-folder-access",
            "--ipc-v1",
            r"\\.\pipe\MTT.FolderAccess.00000000000000000000000000000000",
            "1",
            "S-1-1-0",
        ])
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(87));
}
