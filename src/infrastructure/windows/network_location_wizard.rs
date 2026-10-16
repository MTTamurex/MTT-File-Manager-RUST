//! Launch the Windows Add Network Location wizard and wait off the UI thread.

use std::ffi::OsString;
use std::io;
use std::os::windows::{ffi::OsStringExt, process::CommandExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use windows::Win32::System::SystemInformation::GetSystemDirectoryW;

fn wizard_command_parts(system_directory: &Path) -> (PathBuf, OsString) {
    let executable = system_directory.join("rundll32.exe");
    let mut parameters = OsString::from("\"");
    parameters.push(system_directory.join("shwebsvc.dll"));
    parameters.push("\",AddNetPlaceRunDll");
    (executable, parameters)
}

pub(crate) fn run_network_location_wizard() -> io::Result<()> {
    let mut directory = vec![0u16; 32_768];
    let len = unsafe { GetSystemDirectoryW(Some(&mut directory)) } as usize;
    if len == 0 {
        return Err(io::Error::last_os_error());
    }
    if len >= directory.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Windows system directory is too long",
        ));
    }
    let directory = PathBuf::from(OsString::from_wide(&directory[..len]));
    let (executable, parameters) = wizard_command_parts(&directory);
    // Rundll32 expects the DLL path quoted separately from its entry point.
    // Both paths are resolved from Windows, and no elevation verb is requested.
    Command::new(executable).raw_arg(parameters).status()?;
    // The DLL entry point returns void; closing/cancelling the wizard is normal.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_location_wizard_quotes_the_local_module_independently_of_its_entry_point() {
        let (executable, parameters) = wizard_command_parts(Path::new(r"C:\System Directory"));
        assert_eq!(
            executable,
            PathBuf::from(r"C:\System Directory\rundll32.exe")
        );
        assert_eq!(
            parameters,
            OsString::from(r#""C:\System Directory\shwebsvc.dll",AddNetPlaceRunDll"#)
        );
    }
}
