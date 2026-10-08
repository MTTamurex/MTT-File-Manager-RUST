#![cfg(target_os = "windows")]

use std::ffi::c_void;
use std::mem::size_of;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::process::Command;
use std::ptr;
use std::sync::atomic::{AtomicU32, Ordering};
use windows::core::{
    implement, Error, IUnknown, Interface, Ref, BOOL, GUID, HRESULT, PCWSTR, PWSTR,
};
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, WIN32_ERROR};
use windows::Win32::System::Com::{
    CoTaskMemAlloc, CoTaskMemFree, IBindCtx, IClassFactory, IClassFactory_Impl,
};
use windows::Win32::System::Registry::{
    RegGetValueW, HKEY_CURRENT_USER, REG_SZ, REG_VALUE_TYPE, RRF_RT_ANY,
};
use windows::Win32::UI::Shell::{
    IEnumExplorerCommand, IExecuteCommand, IExplorerCommand, IExplorerCommand_Impl,
    IObjectWithSelection, IShellItemArray, ECF_DEFAULT, ECS_ENABLED, ECS_HIDDEN, SIGDN_FILESYSPATH,
};

mod folder_delegate;
mod local_server;
use folder_delegate::{FolderOpenCommand, CLSID_MTT_FOLDER_DELEGATE};

pub use local_server::run_local_server;

const CLSID_MTT_EXPLORER_COMMAND: GUID = GUID::from_u128(0x9f2265b8_8c7a_4b65_93a7_fae34ccb0347);
const MENU_KEY: &str = "Software\\Classes\\Directory\\shell\\MTT.FileManager.OpenInMTT";
const CONTEXT_MENU_STATE_KEY: &str = "Software\\MTT-File-Manager\\ShellIntegration\\ContextMenu";
const FALLBACK_TITLE: &str = "Open in MTT";
const MAX_SHELL_PATH_UNITS: usize = 32767;
const S_OK: HRESULT = HRESULT(0);
const S_FALSE: HRESULT = HRESULT(1);
const E_POINTER_HR: HRESULT = HRESULT(0x80004003u32 as i32);
const E_NOINTERFACE_HR: HRESULT = HRESULT(0x80004002u32 as i32);
const E_NOTIMPL_HR: HRESULT = HRESULT(0x80004001u32 as i32);
const E_FAIL_HR: HRESULT = HRESULT(0x80004005u32 as i32);
const E_OUTOFMEMORY_HR: HRESULT = HRESULT(0x8007000eu32 as i32);
const E_INVALIDARG_HR: HRESULT = HRESULT(0x80070057u32 as i32);
const CLASS_E_NOAGGREGATION_HR: HRESULT = HRESULT(0x80040110u32 as i32);
const CLASS_E_CLASSNOTAVAILABLE_HR: HRESULT = HRESULT(0x80040111u32 as i32);

static LIVE_OBJECTS: AtomicU32 = AtomicU32::new(0);
static LIVE_SERVER_COMMANDS: AtomicU32 = AtomicU32::new(0);
static SERVER_LOCKS: AtomicU32 = AtomicU32::new(0);
static COMMANDS_SERVED: AtomicU32 = AtomicU32::new(0);
static LAST_COMMAND_ACTIVITY: AtomicU32 = AtomicU32::new(0);

fn current_tick_milliseconds() -> u32 {
    unsafe { windows::Win32::System::SystemInformation::GetTickCount() }
}

fn record_command_served() {
    LIVE_SERVER_COMMANDS.fetch_add(1, Ordering::SeqCst);
    COMMANDS_SERVED.fetch_add(1, Ordering::SeqCst);
    LAST_COMMAND_ACTIVITY.store(current_tick_milliseconds(), Ordering::SeqCst);
}

fn record_command_release() {
    LAST_COMMAND_ACTIVITY.store(current_tick_milliseconds(), Ordering::SeqCst);
    LIVE_SERVER_COMMANDS.fetch_sub(1, Ordering::SeqCst);
}

fn win32_hresult(code: WIN32_ERROR) -> HRESULT {
    if code.0 == 0 {
        S_OK
    } else {
        HRESULT(((code.0 & 0xffff) | 0x80070000) as i32)
    }
}

fn read_registry_string(
    key_path: &str,
    value_name: Option<&str>,
) -> windows::core::Result<Option<String>> {
    let key_wide: Vec<u16> = key_path.encode_utf16().chain(std::iter::once(0)).collect();
    let value_name_wide = value_name.map(|name| {
        name.encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>()
    });
    let value_name_pointer = value_name_wide
        .as_ref()
        .map_or_else(PCWSTR::null, |name| PCWSTR(name.as_ptr()));
    let mut value_type = REG_VALUE_TYPE(0);
    let mut size = 0u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key_wide.as_ptr()),
            value_name_pointer,
            RRF_RT_ANY,
            Some(&mut value_type),
            None,
            Some(&mut size),
        )
    };
    if status.0 == ERROR_FILE_NOT_FOUND.0 || status.0 == ERROR_PATH_NOT_FOUND.0 {
        return Ok(None);
    }
    if !status.is_ok() {
        return Err(Error::from_hresult(win32_hresult(status)));
    }
    if value_type != REG_SZ {
        return Ok(None);
    }

    let mut bytes = vec![0; size as usize];
    if size > 0 {
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                PCWSTR(key_wide.as_ptr()),
                value_name_pointer,
                RRF_RT_ANY,
                Some(&mut value_type),
                Some(bytes.as_mut_ptr().cast()),
                Some(&mut size),
            )
        };
        if !status.is_ok() {
            return Err(Error::from_hresult(win32_hresult(status)));
        }
        bytes.truncate(size as usize);
    }

    let mut units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    while units.last() == Some(&0) {
        units.pop();
    }
    String::from_utf16(&units)
        .map(Some)
        .map_err(|_| Error::from_hresult(E_FAIL_HR))
}

fn allocate_wide(value: &str) -> windows::core::Result<PWSTR> {
    let units: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = units.len().saturating_mul(size_of::<u16>());
    let allocation = unsafe { CoTaskMemAlloc(bytes) };
    if allocation.is_null() {
        return Err(Error::from_hresult(E_OUTOFMEMORY_HR));
    }
    unsafe {
        ptr::copy_nonoverlapping(units.as_ptr(), allocation.cast::<u16>(), units.len());
    }
    Ok(PWSTR(allocation.cast()))
}

fn strip_command_quotes(command: &str) -> &str {
    let trimmed = command.trim();
    match trimmed.strip_prefix('"') {
        Some(rest) => rest.split('"').next().unwrap_or(rest),
        None => trimmed.split(' ').next().unwrap_or(trimmed),
    }
}

fn command_is_enabled() -> bool {
    matches!(
        read_registry_string(CONTEXT_MENU_STATE_KEY, None),
        Ok(Some(_))
    )
}

#[implement(IExplorerCommand)]
struct ExplorerCommand;

impl ExplorerCommand {
    fn new() -> Self {
        LIVE_OBJECTS.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

impl Drop for ExplorerCommand {
    fn drop(&mut self) {
        LIVE_OBJECTS.fetch_sub(1, Ordering::SeqCst);
    }
}

#[allow(non_snake_case)]
impl IExplorerCommand_Impl for ExplorerCommand_Impl {
    fn GetTitle(&self, _items: Ref<IShellItemArray>) -> windows::core::Result<PWSTR> {
        let title =
            read_registry_string(MENU_KEY, None)?.unwrap_or_else(|| FALLBACK_TITLE.to_string());
        allocate_wide(&title)
    }

    fn GetIcon(&self, _items: Ref<IShellItemArray>) -> windows::core::Result<PWSTR> {
        let icon = read_registry_string(MENU_KEY, Some("Icon"))?
            .ok_or_else(|| Error::from_hresult(E_NOTIMPL_HR))?;
        allocate_wide(&icon)
    }

    fn GetToolTip(&self, _items: Ref<IShellItemArray>) -> windows::core::Result<PWSTR> {
        Err(Error::from_hresult(E_NOTIMPL_HR))
    }

    fn GetCanonicalName(&self) -> windows::core::Result<GUID> {
        Ok(CLSID_MTT_EXPLORER_COMMAND)
    }

    fn GetState(
        &self,
        items: Ref<IShellItemArray>,
        _ok_to_be_slow: BOOL,
    ) -> windows::core::Result<u32> {
        if !command_is_enabled() {
            return Ok(ECS_HIDDEN.0 as u32);
        }
        let Some(items) = items.as_ref() else {
            return Ok(ECS_HIDDEN.0 as u32);
        };
        if unsafe { items.GetCount()? } != 1 {
            return Ok(ECS_HIDDEN.0 as u32);
        }
        Ok(ECS_ENABLED.0 as u32)
    }

    fn Invoke(
        &self,
        items: Ref<IShellItemArray>,
        _bind_ctx: Ref<IBindCtx>,
    ) -> windows::core::Result<()> {
        if !command_is_enabled() {
            return Err(Error::from_hresult(E_FAIL_HR));
        }
        let items = items.ok()?;
        if unsafe { items.GetCount()? } != 1 {
            return Err(Error::from_hresult(E_INVALIDARG_HR));
        }

        let item = unsafe { items.GetItemAt(0)? };
        let display_name = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH)? };
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
        let path = PathBuf::from(std::ffi::OsString::from_wide(unsafe {
            std::slice::from_raw_parts(display_name.0 .0, length)
        }));
        if !path.is_absolute() {
            return Err(Error::from_hresult(E_INVALIDARG_HR));
        }

        let executable = read_registry_string(CONTEXT_MENU_STATE_KEY, None)?
            .ok_or_else(|| Error::from_hresult(E_FAIL_HR))?;
        Command::new(executable)
            .arg(path)
            .spawn()
            .map_err(|_| Error::from_hresult(E_FAIL_HR))?;
        Ok(())
    }

    fn GetFlags(&self) -> windows::core::Result<u32> {
        Ok(ECF_DEFAULT.0 as u32)
    }

    fn EnumSubCommands(&self) -> windows::core::Result<IEnumExplorerCommand> {
        Err(Error::from_hresult(E_NOTIMPL_HR))
    }
}

struct ShellAllocatedString(PWSTR);

impl Drop for ShellAllocatedString {
    fn drop(&mut self) {
        unsafe {
            CoTaskMemFree(Some(self.0 .0.cast()));
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClassKind {
    ExplorerCommand,
    FolderDelegate,
}

fn class_kind_for_id(class_id: GUID) -> Option<ClassKind> {
    match class_id {
        CLSID_MTT_EXPLORER_COMMAND => Some(ClassKind::ExplorerCommand),
        CLSID_MTT_FOLDER_DELEGATE => Some(ClassKind::FolderDelegate),
        _ => None,
    }
}

#[implement(IClassFactory)]
struct ClassFactory {
    kind: ClassKind,
}

impl ClassFactory {
    fn new(kind: ClassKind) -> Self {
        LIVE_OBJECTS.fetch_add(1, Ordering::SeqCst);
        Self { kind }
    }
}

impl Drop for ClassFactory {
    fn drop(&mut self) {
        LIVE_OBJECTS.fetch_sub(1, Ordering::SeqCst);
    }
}

#[allow(non_snake_case)]
impl IClassFactory_Impl for ClassFactory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<IUnknown>,
        riid: *const GUID,
        object: *mut *mut c_void,
    ) -> windows::core::Result<()> {
        if object.is_null() {
            return Err(Error::from_hresult(E_POINTER_HR));
        }
        unsafe { *object = ptr::null_mut() };
        if riid.is_null() {
            return Err(Error::from_hresult(E_POINTER_HR));
        }
        if !outer.is_null() {
            return Err(Error::from_hresult(CLASS_E_NOAGGREGATION_HR));
        }
        let requested_iid = unsafe { *riid };
        match self.kind {
            ClassKind::ExplorerCommand => {
                if requested_iid != IExplorerCommand::IID && requested_iid != IUnknown::IID {
                    return Err(Error::from_hresult(E_NOINTERFACE_HR));
                }
                let command: IExplorerCommand = ExplorerCommand::new().into();
                unsafe { *object = command.as_raw() };
                std::mem::forget(command);
            }
            ClassKind::FolderDelegate => {
                if requested_iid == IExecuteCommand::IID {
                    let command: IExecuteCommand = FolderOpenCommand::new().into();
                    unsafe { *object = command.as_raw() };
                    std::mem::forget(command);
                } else if requested_iid == IObjectWithSelection::IID {
                    let command: IObjectWithSelection = FolderOpenCommand::new().into();
                    unsafe { *object = command.as_raw() };
                    std::mem::forget(command);
                } else if requested_iid == IUnknown::IID {
                    let command: IUnknown = FolderOpenCommand::new().into();
                    unsafe { *object = command.as_raw() };
                    std::mem::forget(command);
                } else {
                    return Err(Error::from_hresult(E_NOINTERFACE_HR));
                }
            }
        }
        Ok(())
    }

    fn LockServer(&self, lock: BOOL) -> windows::core::Result<()> {
        if lock.as_bool() {
            SERVER_LOCKS.fetch_add(1, Ordering::SeqCst);
        } else {
            let _ = SERVER_LOCKS.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
                Some(count.saturating_sub(1))
            });
        }
        Ok(())
    }
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "system" fn DllGetClassObject(
    class_id: *const GUID,
    interface_id: *const GUID,
    object: *mut *mut c_void,
) -> HRESULT {
    if object.is_null() {
        return E_POINTER_HR;
    }
    unsafe { *object = ptr::null_mut() };
    if class_id.is_null() || interface_id.is_null() {
        return E_POINTER_HR;
    }
    let Some(class_kind) = class_kind_for_id(unsafe { *class_id }) else {
        return CLASS_E_CLASSNOTAVAILABLE_HR;
    };
    let requested_iid = unsafe { *interface_id };
    if requested_iid != IClassFactory::IID && requested_iid != IUnknown::IID {
        return E_NOINTERFACE_HR;
    }
    let factory: IClassFactory = ClassFactory::new(class_kind).into();
    unsafe { *object = factory.as_raw() };
    std::mem::forget(factory);
    S_OK
}

#[no_mangle]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    if LIVE_OBJECTS.load(Ordering::SeqCst) == 0 && SERVER_LOCKS.load(Ordering::SeqCst) == 0 {
        S_OK
    } else {
        S_FALSE
    }
}

#[cfg(test)]
mod tests {
    use super::{
        class_kind_for_id, strip_command_quotes, ClassKind, CLSID_MTT_EXPLORER_COMMAND,
        CLSID_MTT_FOLDER_DELEGATE,
    };

    #[test]
    fn class_factory_keeps_both_shell_classes_distinct() {
        assert_eq!(
            class_kind_for_id(CLSID_MTT_EXPLORER_COMMAND),
            Some(ClassKind::ExplorerCommand)
        );
        assert_eq!(
            class_kind_for_id(CLSID_MTT_FOLDER_DELEGATE),
            Some(ClassKind::FolderDelegate)
        );
    }

    #[test]
    fn extracts_the_executable_from_a_local_server_command() {
        assert_eq!(
            strip_command_quotes(
                r#""C:\Program Files\MTT File Manager\mtt-shell-command.exe" -Embedding"#
            ),
            r"C:\Program Files\MTT File Manager\mtt-shell-command.exe"
        );
        assert_eq!(
            strip_command_quotes(r"C:\MTT\mtt-shell-command.exe -Embedding"),
            r"C:\MTT\mtt-shell-command.exe"
        );
    }
}
