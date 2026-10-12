use std::mem::size_of;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::process::Command;
use std::ptr;
use std::sync::atomic::Ordering;

use windows::core::{implement, Error, Ref, BOOL, GUID, PWSTR};
use windows::Win32::System::Com::{CoTaskMemAlloc, CoTaskMemFree, IBindCtx};
use windows::Win32::UI::Shell::{
    IEnumExplorerCommand, IExplorerCommand, IExplorerCommand_Impl, IShellItemArray, ECF_DEFAULT,
    ECS_ENABLED, ECS_HIDDEN, SIGDN_FILESYSPATH,
};

use crate::registry;
use crate::{
    CLSID_MTT_EXPLORER_COMMAND, E_FAIL_HR, E_INVALIDARG_HR, E_NOTIMPL_HR, E_OUTOFMEMORY_HR,
    LIVE_OBJECTS,
};

const MAX_SHELL_PATH_UNITS: usize = 32_767;

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

#[implement(IExplorerCommand)]
pub(super) struct OpenInMttCommand;

impl OpenInMttCommand {
    pub(super) fn new() -> Self {
        LIVE_OBJECTS.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

impl Drop for OpenInMttCommand {
    fn drop(&mut self) {
        LIVE_OBJECTS.fetch_sub(1, Ordering::SeqCst);
    }
}

#[allow(non_snake_case)]
impl IExplorerCommand_Impl for OpenInMttCommand_Impl {
    fn GetTitle(&self, _items: Ref<IShellItemArray>) -> windows::core::Result<PWSTR> {
        allocate_wide(&registry::menu_title()?)
    }

    fn GetIcon(&self, _items: Ref<IShellItemArray>) -> windows::core::Result<PWSTR> {
        let icon = registry::menu_icon()?.ok_or_else(|| Error::from_hresult(E_NOTIMPL_HR))?;
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
        if !registry::context_menu_is_enabled() {
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
        if !registry::context_menu_is_enabled() {
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

        let executable = registry::context_menu_executable()?;
        Command::new(executable)
            .arg("--open-folder")
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
