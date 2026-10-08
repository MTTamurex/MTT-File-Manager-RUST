use std::ffi::c_void;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;
use std::process::Command;
use std::ptr;
use std::sync::Mutex;
use windows::core::{implement, Error, IUnknown, Interface, Ref, BOOL, GUID, PCWSTR};
use windows::Win32::Foundation::POINT;
use windows::Win32::UI::Shell::{
    IExecuteCommand, IExecuteCommand_Impl, IObjectWithSelection, IObjectWithSelection_Impl,
    IShellItem, IShellItemArray, SIGDN_DESKTOPABSOLUTEPARSING, SIGDN_FILESYSPATH,
};

use super::{
    record_command_release, record_command_served, ShellAllocatedString, E_FAIL_HR,
    E_INVALIDARG_HR, E_NOINTERFACE_HR, E_POINTER_HR, LIVE_OBJECTS, MAX_SHELL_PATH_UNITS,
};

pub(crate) const CLSID_MTT_FOLDER_DELEGATE: GUID =
    GUID::from_u128(0x2e6a41d7_54b3_4f0e_b8c2_9d17a3e5c6b8);

const LOCAL_SERVER_KEY_PATH: &str =
    "Software\\Classes\\CLSID\\{2e6a41d7-54b3-4f0e-b8c2-9d17a3e5c6b8}\\LocalServer32";
const SERVER_EXECUTABLE_NAME: &str = "mtt-shell-command.exe";
const APP_EXECUTABLE_NAME: &str = "mtt-file-manager.exe";

#[implement(IExecuteCommand, IObjectWithSelection)]
pub(crate) struct FolderOpenCommand {
    selection: Mutex<Option<IShellItemArray>>,
    parameters: Mutex<Option<std::ffi::OsString>>,
}

impl FolderOpenCommand {
    pub(crate) fn new() -> Self {
        LIVE_OBJECTS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        record_command_served();
        Self {
            selection: Mutex::new(None),
            parameters: Mutex::new(None),
        }
    }
}

impl Drop for FolderOpenCommand {
    fn drop(&mut self) {
        record_command_release();
        LIVE_OBJECTS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

fn mutex_error() -> Error {
    Error::from_hresult(E_FAIL_HR)
}

fn os_string_from_wide(pointer: *const u16) -> windows::core::Result<Option<std::ffi::OsString>> {
    if pointer.is_null() {
        return Ok(None);
    }
    let mut length = 0usize;
    while length < MAX_SHELL_PATH_UNITS && unsafe { *pointer.add(length) } != 0 {
        length += 1;
    }
    if length == MAX_SHELL_PATH_UNITS {
        return Err(Error::from_hresult(E_INVALIDARG_HR));
    }
    let units = unsafe { std::slice::from_raw_parts(pointer, length) };
    Ok(Some(std::ffi::OsString::from_wide(units)))
}

fn strip_wrapping_quotes(value: std::ffi::OsString) -> std::ffi::OsString {
    let units: Vec<u16> = value.encode_wide().collect();
    if units.len() >= 2 && units[0] == b'"' as u16 && units.last() == Some(&(b'"' as u16)) {
        std::ffi::OsString::from_wide(&units[1..units.len() - 1])
    } else {
        value
    }
}

fn shell_item_name(
    item: &IShellItem,
    kind: windows::Win32::UI::Shell::SIGDN,
) -> windows::core::Result<std::ffi::OsString> {
    let display_name = unsafe { item.GetDisplayName(kind)? };
    if display_name.0.is_null() {
        return Err(Error::from_hresult(E_FAIL_HR));
    }
    let display_name = ShellAllocatedString(display_name);
    let mut length = 0usize;
    while length < MAX_SHELL_PATH_UNITS && unsafe { *display_name.0 .0.add(length) } != 0 {
        length += 1;
    }
    if length == MAX_SHELL_PATH_UNITS {
        return Err(Error::from_hresult(E_INVALIDARG_HR));
    }
    let units = unsafe { std::slice::from_raw_parts(display_name.0 .0, length) };
    Ok(std::ffi::OsString::from_wide(units))
}

fn mtt_executable() -> windows::core::Result<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(server) = std::env::current_exe() {
        if let Some(directory) = server.parent() {
            candidates.push(directory.join(APP_EXECUTABLE_NAME));
        }
    }
    if let Ok(Some(server_path)) = super::read_registry_string(LOCAL_SERVER_KEY_PATH, None) {
        let server = PathBuf::from(super::strip_command_quotes(&server_path));
        if server
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.eq_ignore_ascii_case(SERVER_EXECUTABLE_NAME))
            .unwrap_or(false)
        {
            if let Some(directory) = server.parent() {
                candidates.push(directory.join(APP_EXECUTABLE_NAME));
            }
        }
    }
    candidates
        .into_iter()
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| Error::from_hresult(E_FAIL_HR))
}

fn launch_mtt(path: PathBuf) -> windows::core::Result<()> {
    Command::new(mtt_executable()?)
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|_| Error::from_hresult(E_FAIL_HR))
}

fn launch_explorer(parsing_name: std::ffi::OsString) -> windows::core::Result<()> {
    let explorer = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .map(|root| root.join("explorer.exe"))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from("explorer.exe"));
    Command::new(explorer)
        .arg(parsing_name)
        .spawn()
        .map(|_| ())
        .map_err(|_| Error::from_hresult(E_FAIL_HR))
}

#[allow(non_snake_case)]
impl IObjectWithSelection_Impl for FolderOpenCommand_Impl {
    fn SetSelection(&self, selection: Ref<IShellItemArray>) -> windows::core::Result<()> {
        *self.selection.lock().map_err(|_| mutex_error())? = selection.as_ref().cloned();
        Ok(())
    }

    fn GetSelection(
        &self,
        riid: *const GUID,
        object: *mut *mut c_void,
    ) -> windows::core::Result<()> {
        if riid.is_null() || object.is_null() {
            return Err(Error::from_hresult(E_POINTER_HR));
        }
        unsafe { *object = ptr::null_mut() };
        let requested_iid = unsafe { *riid };
        if requested_iid != IShellItemArray::IID && requested_iid != IUnknown::IID {
            return Err(Error::from_hresult(E_NOINTERFACE_HR));
        }
        let selection = self
            .selection
            .lock()
            .map_err(|_| mutex_error())?
            .clone()
            .ok_or_else(|| Error::from_hresult(E_FAIL_HR))?;
        if requested_iid == IShellItemArray::IID {
            unsafe { *object = selection.as_raw() };
            std::mem::forget(selection);
        } else {
            let unknown: IUnknown = selection.cast()?;
            unsafe { *object = unknown.as_raw() };
            std::mem::forget(unknown);
        }
        Ok(())
    }
}

#[allow(non_snake_case)]
impl IExecuteCommand_Impl for FolderOpenCommand_Impl {
    fn SetKeyState(&self, _key_state: u32) -> windows::core::Result<()> {
        Ok(())
    }

    fn SetParameters(&self, parameters: &PCWSTR) -> windows::core::Result<()> {
        *self.parameters.lock().map_err(|_| mutex_error())? =
            os_string_from_wide(parameters.0)?.map(strip_wrapping_quotes);
        Ok(())
    }

    fn SetPosition(&self, _position: &POINT) -> windows::core::Result<()> {
        Ok(())
    }

    fn SetShowWindow(&self, _show_window: i32) -> windows::core::Result<()> {
        Ok(())
    }

    fn SetNoShowUI(&self, _no_show_ui: BOOL) -> windows::core::Result<()> {
        Ok(())
    }

    fn SetDirectory(&self, _directory: &PCWSTR) -> windows::core::Result<()> {
        Ok(())
    }

    fn Execute(&self) -> windows::core::Result<()> {
        if let Some(selection) = self.selection.lock().map_err(|_| mutex_error())?.clone() {
            let count = unsafe { selection.GetCount()? };
            if count > 1 {
                return Err(Error::from_hresult(E_INVALIDARG_HR));
            }
            if count == 1 {
                let item = unsafe { selection.GetItemAt(0)? };
                if let Ok(path) = shell_item_name(&item, SIGDN_FILESYSPATH) {
                    let path = PathBuf::from(path);
                    if path.is_absolute() {
                        return launch_mtt(path);
                    }
                }
                return launch_explorer(shell_item_name(&item, SIGDN_DESKTOPABSOLUTEPARSING)?);
            }
        }

        let parameters = self
            .parameters
            .lock()
            .map_err(|_| mutex_error())?
            .clone()
            .ok_or_else(|| Error::from_hresult(E_INVALIDARG_HR))?;
        let path = PathBuf::from(parameters);
        if path.is_absolute() {
            launch_mtt(path)
        } else {
            launch_explorer(path.into_os_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::strip_wrapping_quotes;
    use std::ffi::OsString;

    #[test]
    fn strips_shell_argument_quotes_without_changing_unicode_paths() {
        assert_eq!(
            strip_wrapping_quotes(OsString::from(r#""C:\Program Files\MTT 文件夹""#)),
            OsString::from(r"C:\Program Files\MTT 文件夹")
        );
    }
}
