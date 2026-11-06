//! Independent Windows controls plus the production rejection-only lookup.

use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};

use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_NO_MORE_FILES, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

use crate::ipc_authorization::with_restricting_sid;

use super::{inspect_pid, BasenameEvidence};

fn own_snapshot_name() -> Result<String, &'static str> {
    let handle = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }
        .map_err(|_| "cannot create process snapshot")?;
    let handle = unsafe { OwnedHandle::from_raw_handle(handle.0) };
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    unsafe { Process32FirstW(HANDLE(handle.as_raw_handle()), &mut entry) }
        .map_err(|_| "cannot start process enumeration")?;
    // Do not collect or log the other processes' entries. Only return our own
    // test process's basename; the snapshot is closed on every exit.
    for _ in 0..65536 {
        if entry.th32ProcessID == std::process::id() {
            let length = entry
                .szExeFile
                .iter()
                .position(|unit| *unit == 0)
                .ok_or("unterminated snapshot basename")?;
            if length == 0 {
                return Err("missing snapshot basename");
            }
            return String::from_utf16(&entry.szExeFile[..length])
                .map_err(|_| "invalid snapshot basename");
        }
        match unsafe { Process32NextW(HANDLE(handle.as_raw_handle()), &mut entry) } {
            Ok(()) => {}
            Err(error) if error.code() == ERROR_NO_MORE_FILES.to_hresult() => {
                return Err("test process was not found in snapshot");
            }
            Err(_) => return Err("process enumeration failed"),
        }
    }
    Err("process enumeration exceeded the safety bound")
}

fn assert_matches_own_image_without_logging_names(name: &str) {
    let path = std::env::current_exe().unwrap();
    let expected = path.file_name().unwrap().to_str().unwrap();
    assert!(name.eq_ignore_ascii_case(expected));
    assert!(!name.eq_ignore_ascii_case("mtt-file-manager.exe"));
}

#[test]
fn snapshot_identifies_the_own_non_mtt_basename_in_normal_context() {
    let name = own_snapshot_name().unwrap();
    assert_matches_own_image_without_logging_names(&name);
    assert_eq!(
        inspect_pid(std::process::id()),
        Ok(BasenameEvidence::Foreign)
    );
}

#[test]
fn snapshot_can_still_identify_own_basename_when_process_open_is_denied() {
    let name = with_restricting_sid(|| {
        match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, std::process::id()) } {
            Err(error) => assert_eq!(error.code(), ERROR_ACCESS_DENIED.to_hresult()),
            Ok(handle) => {
                let _handle = unsafe { OwnedHandle::from_raw_handle(handle.0) };
                panic!("fixture did not deny the normal process query");
            }
        }
        own_snapshot_name().unwrap()
    });
    assert_matches_own_image_without_logging_names(&name);
    with_restricting_sid(|| {
        assert_eq!(
            inspect_pid(std::process::id()),
            Ok(BasenameEvidence::Foreign)
        );
    });
}
