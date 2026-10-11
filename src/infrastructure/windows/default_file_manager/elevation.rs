use std::ffi::OsString;
use std::io::Write;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_CANCELLED, HWND, WAIT_OBJECT_0};
use windows::Win32::System::SystemInformation::GetWindowsDirectoryW;
use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
use windows::Win32::UI::Shell::{
    ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

use super::RegistryActionError;

pub(super) fn import_registry_script(
    script: &str,
    owner_window: Option<isize>,
) -> Result<(), RegistryActionError> {
    let mut temporary_file = tempfile::Builder::new()
        .prefix("mtt-file-manager-default-")
        .suffix(".reg")
        .tempfile()
        .map_err(|_| {
            log::error!("[DEFAULT-FILE-MANAGER] Failed to create temporary registry script");
            RegistryActionError::Failed
        })?;
    let windows_lines = script.replace('\n', "\r\n");
    let mut contents = vec![0xff, 0xfe];
    for unit in windows_lines.encode_utf16() {
        contents.extend_from_slice(&unit.to_le_bytes());
    }
    temporary_file
        .write_all(&contents)
        .and_then(|_| temporary_file.flush())
        .map_err(|_| {
            log::error!("[DEFAULT-FILE-MANAGER] Failed to write temporary registry script");
            RegistryActionError::Failed
        })?;

    // Close the writer before starting the elevated process. In particular,
    // this avoids sharing-mode or integrity-level issues when Regedit opens
    // the temporary script from the user's temp directory.
    let temporary_path = temporary_file.into_temp_path();
    run_elevated_regedit(&temporary_path, owner_window)
}

fn run_elevated_regedit(
    registry_file: &Path,
    owner_window: Option<isize>,
) -> Result<(), RegistryActionError> {
    let Some(regedit) = windows_directory().map(|directory| directory.join("regedit.exe")) else {
        log::error!("[DEFAULT-FILE-MANAGER] Failed to resolve the Windows directory");
        return Err(RegistryActionError::Failed);
    };
    if !regedit.is_file() {
        log::error!(
            "[DEFAULT-FILE-MANAGER] Registry Editor was not found in the Windows directory"
        );
        return Err(RegistryActionError::Failed);
    }
    let executable: Vec<u16> = regedit.as_os_str().encode_wide().chain(Some(0)).collect();
    let verb: Vec<u16> = "runas".encode_utf16().chain(Some(0)).collect();
    let parameters = regedit_parameters(registry_file);
    let mut exec_info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_FLAG_NO_UI,
        hwnd: owner_window
            .map(|window| HWND(window as *mut _))
            .unwrap_or_default(),
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(executable.as_ptr()),
        lpParameters: PCWSTR(parameters.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };

    unsafe {
        if ShellExecuteExW(&mut exec_info).is_err() {
            let error = GetLastError();
            if error == ERROR_CANCELLED {
                return Err(RegistryActionError::Cancelled);
            }
            log::error!(
                "[DEFAULT-FILE-MANAGER] ShellExecuteExW failed to start elevated Registry Editor: {error:?}"
            );
            return Err(RegistryActionError::Failed);
        }

        let process = exec_info.hProcess;
        if process.is_invalid() {
            log::error!(
                "[DEFAULT-FILE-MANAGER] Elevated Registry Editor returned no process handle"
            );
            return Err(RegistryActionError::Failed);
        }

        let wait_result = WaitForSingleObject(process, u32::MAX);
        let mut exit_code = 1u32;
        let exit_code_result = GetExitCodeProcess(process, &mut exit_code);
        let _ = CloseHandle(process);

        if wait_result != WAIT_OBJECT_0 || exit_code_result.is_err() || exit_code != 0 {
            log::error!(
                "[DEFAULT-FILE-MANAGER] Elevated Registry Editor failed: wait={wait_result:?} exit_code={exit_code}"
            );
            return Err(RegistryActionError::Failed);
        }
    }

    Ok(())
}

fn windows_directory() -> Option<PathBuf> {
    let mut buffer = vec![0u16; 32_768];
    let length = unsafe { GetWindowsDirectoryW(Some(&mut buffer)) } as usize;
    if length == 0 || length >= buffer.len() {
        return None;
    }
    buffer.truncate(length);
    Some(PathBuf::from(OsString::from_wide(&buffer)))
}

fn regedit_parameters(registry_file: &Path) -> Vec<u16> {
    let mut parameters: Vec<u16> = "/s ".encode_utf16().collect();
    let argument: Vec<u16> = registry_file.as_os_str().encode_wide().collect();
    append_quoted_argument(&mut parameters, &argument);
    parameters.push(0);
    parameters
}

fn append_quoted_argument(output: &mut Vec<u16>, argument: &[u16]) {
    output.push(b'"' as u16);
    let mut backslashes = 0usize;
    for unit in argument {
        if *unit == b'\\' as u16 {
            backslashes += 1;
        } else if *unit == b'"' as u16 {
            output.extend(std::iter::repeat_n(b'\\' as u16, backslashes * 2 + 1));
            output.push(*unit);
            backslashes = 0;
        } else {
            output.extend(std::iter::repeat_n(b'\\' as u16, backslashes));
            output.push(*unit);
            backslashes = 0;
        }
    }
    output.extend(std::iter::repeat_n(b'\\' as u16, backslashes * 2));
    output.push(b'"' as u16);
}

#[cfg(test)]
mod tests {
    use super::append_quoted_argument;

    #[test]
    fn quoted_regedit_argument_preserves_spaces_and_trailing_separator_rules() {
        let mut output = Vec::new();
        append_quoted_argument(
            &mut output,
            r"C:\Program Files\Example.reg"
                .encode_utf16()
                .collect::<Vec<_>>()
                .as_slice(),
        );
        assert_eq!(
            String::from_utf16(&output).unwrap(),
            r#""C:\Program Files\Example.reg""#
        );
    }
}
