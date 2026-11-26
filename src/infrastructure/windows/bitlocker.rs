//! BitLocker locked-volume detection and unlock launcher.

use std::io;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::process::Command;
use windows::core::HRESULT;
use windows::Win32::System::SystemInformation::GetSystemDirectoryW;

/// `FVE_E_LOCKED_VOLUME`: the volume is BitLocker-locked and not accessible.
pub const FVE_E_LOCKED_VOLUME: HRESULT = HRESULT(0x8031_0000_u32 as i32);

/// Returns true when the error code means a BitLocker-locked volume.
pub fn is_locked_volume_error(code: HRESULT) -> bool {
    code == FVE_E_LOCKED_VOLUME
}

fn system_directory() -> io::Result<PathBuf> {
    let mut buffer = [0u16; 260];
    let len = unsafe { GetSystemDirectoryW(Some(&mut buffer)) } as usize;
    if len == 0 || len >= buffer.len() {
        return Err(io::Error::last_os_error());
    }
    Ok(PathBuf::from(std::ffi::OsString::from_wide(&buffer[..len])))
}

/// Launches the Windows BitLocker unlock wizard for a drive root such as `E:\`.
pub fn unlock_drive(root: &str) -> io::Result<()> {
    let letter = root
        .chars()
        .next()
        .filter(|c| c.is_ascii_alphabetic())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid drive root"))?;
    let system = system_directory()?;
    Command::new(system.join("bdeunlock.exe"))
        .arg(format!("{}:", letter.to_ascii_uppercase()))
        .current_dir(&system)
        .spawn()
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_locked_volume_code() {
        assert!(is_locked_volume_error(HRESULT(0x8031_0000_u32 as i32)));
        assert!(!is_locked_volume_error(HRESULT(0x8007_0005_u32 as i32)));
    }

    #[test]
    fn rejects_invalid_drive_root() {
        assert!(unlock_drive("").is_err());
        assert!(unlock_drive("\\\\server\\share").is_err());
    }
}
