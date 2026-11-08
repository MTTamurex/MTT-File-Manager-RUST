use super::{shell_parsing_name, Pidl};
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::Path;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};

#[test]
fn canonical_existing_path_can_be_parsed_by_the_shell() {
    // Parsing the running test executable is read-only: no transfer, third-party
    // drop target, or clipboard operation is invoked.
    let path = std::env::current_exe()
        .expect("get the test executable")
        .canonicalize()
        .expect("canonicalize the test executable");
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
        .ok()
        .expect("initialize test COM apartment");
    struct ComGuard;
    impl Drop for ComGuard {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }
    let _com = ComGuard;
    let result = Pidl::parse(&path);
    assert!(
        result.is_ok(),
        "canonical path parsing failed: {:?}",
        result.err()
    );
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

#[test]
fn converts_verbatim_disk_paths_without_altering_the_filename() {
    assert_eq!(
        shell_parsing_name(Path::new(r"\\?\C:\Source\日本語.txt")),
        wide(r"C:\Source\日本語.txt")
    );
}

#[test]
fn converts_verbatim_unc_paths_to_regular_unc_paths() {
    assert_eq!(
        shell_parsing_name(Path::new(r"\\?\UNC\server\share\file.txt")),
        wide(r"\\server\share\file.txt")
    );
}

#[test]
fn leaves_ordinary_paths_and_other_namespaces_unchanged() {
    for name in [
        r"C:\Source\file.txt",
        r"\\server\share\file.txt",
        r"\\?\Volume{00000000-0000-0000-0000-000000000000}\file.txt",
        r"\\.\C:\Source\file.txt",
    ] {
        assert_eq!(shell_parsing_name(Path::new(name)), wide(name));
    }
}

#[test]
fn conversion_preserves_unpaired_utf16_units() {
    let mut units: Vec<u16> = r"\\?\C:\Source\".encode_utf16().collect();
    units.push(0xD800);
    units.extend(".txt".encode_utf16());
    let input = OsString::from_wide(&units);
    let mut expected = units[4..].to_vec();
    expected.push(0);
    assert_eq!(shell_parsing_name(Path::new(&input)), expected);
}
