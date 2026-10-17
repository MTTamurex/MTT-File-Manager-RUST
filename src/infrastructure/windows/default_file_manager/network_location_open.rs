//! Native fallback for Folder Shortcut targets the application does not navigate.

use std::ffi::OsString;
use std::io;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use windows::Win32::System::SystemInformation::GetWindowsDirectoryW;

pub(super) fn open_in_explorer(path: &Path) -> io::Result<()> {
    let mut directory = vec![0u16; 32_768];
    let len = unsafe { GetWindowsDirectoryW(Some(&mut directory)) } as usize;
    if len == 0 {
        return Err(io::Error::last_os_error());
    }
    if len >= directory.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Windows directory is too long",
        ));
    }
    let explorer = PathBuf::from(OsString::from_wide(&directory[..len])).join("explorer.exe");
    Command::new(explorer).arg("/n").arg(path).spawn()?;
    Ok(())
}
