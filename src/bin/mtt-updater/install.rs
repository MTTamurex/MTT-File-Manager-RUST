use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
use windows::Win32::UI::Shell::{
    ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS,
    SHELLEXECUTEINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

pub(super) enum InstallError {
    Failed(&'static str),
    Unmonitored(&'static str),
}

impl InstallError {
    pub(super) fn message(&self) -> &'static str {
        match self {
            Self::Failed(message) | Self::Unmonitored(message) => message,
        }
    }

    pub(super) fn can_restart(&self) -> bool {
        !matches!(self, Self::Unmonitored(_))
    }
}

impl From<&'static str> for InstallError {
    fn from(message: &'static str) -> Self {
        Self::Failed(message)
    }
}

struct ComScope;
impl Drop for ComScope {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

fn open_verified(setup: &Path, digest: &str) -> Result<std::fs::File, &'static str> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows::Win32::Storage::FileSystem::FILE_SHARE_READ;
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ.0)
        .open(setup)
        .map_err(|_| "Could not open the installer.")?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|_| "Could not verify the installer.")?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    if !format!("{:x}", hash.finalize()).eq_ignore_ascii_case(digest) {
        return Err("Installer integrity verification failed.");
    }
    Ok(file)
}

pub(super) fn run(setup: &Path, digest: &str) -> Result<(), InstallError> {
    if super::platform::elevated() {
        return Err("The updater must not run as administrator.".into());
    }
    if super::platform::other_instances(0).map_err(|_| "Could not inspect running applications.")? {
        return Err("Close the other MTT windows and viewers before updating.".into());
    }
    // Keep the verified file protected against replacement through setup exit.
    let _file = open_verified(setup, digest)?;
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
        .ok()
        .map_err(|_| "Could not initialize the Windows Shell.")?;
    let _com = ComScope;
    let path = super::wide(setup.as_os_str());
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        // This thread has no message loop: do not let the Shell defer startup.
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
        lpVerb: w!("open"),
        lpFile: PCWSTR(path.as_ptr()),
        lpParameters: w!(
            "/SILENT /SUPPRESSMSGBOXES /NORESTART /NOCLOSEAPPLICATIONS /NORESTARTAPPLICATIONS"
        ),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    unsafe {
        ShellExecuteExW(&mut info)
            .map_err(|_| "Update cancelled or the installer could not start.")?;
        if info.hProcess.is_invalid() {
            return Err(InstallError::Unmonitored("Could not monitor setup. Wait for the installer to finish before opening MTT manually."));
        }
        let waited = WaitForSingleObject(info.hProcess, INFINITE);
        if waited != WAIT_OBJECT_0 {
            let _ = CloseHandle(info.hProcess);
            return Err(InstallError::Unmonitored("Lost contact with setup. Wait for the installer to finish before opening MTT manually."));
        }
        let mut code = 1;
        let status = GetExitCodeProcess(info.hProcess, &mut code);
        let _ = CloseHandle(info.hProcess);
        if status.is_err() || code != 0 {
            return Err(
                "The update did not complete. You can try again from Settings > About.".into(),
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verified_installer_cannot_be_written_or_replaced_until_released() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("setup.exe");
        std::fs::write(&path, b"installer contents").unwrap();
        let digest = format!("{:x}", Sha256::digest(b"installer contents"));
        let verified = open_verified(&path, &digest).unwrap();
        assert!(std::fs::OpenOptions::new().write(true).open(&path).is_err());
        assert!(std::fs::remove_file(&path).is_err());
        drop(verified);
        std::fs::write(&path, b"altered contents").unwrap();
        assert!(open_verified(&path, &digest).is_err());
    }

    #[test]
    fn unmonitored_installer_does_not_allow_a_premature_app_restart() {
        assert!(!InstallError::Unmonitored("unknown").can_restart());
        assert!(InstallError::Failed("cancelled").can_restart());
    }
}
