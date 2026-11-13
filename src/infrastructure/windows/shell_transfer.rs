//! Delegate user transfers to the destination folder's native Shell drop target.
//! A return from Drop is a handoff, NOT a transfer-completion notification.

mod paths;
mod registration;
#[cfg(test)]
mod tests;

use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use windows::core::{Interface, PCWSTR};
use windows::Win32::Foundation::{HWND, POINTL};
use windows::Win32::System::Com::{CoTaskMemFree, IDataObject};
use windows::Win32::System::Ole::{
    IDropTarget, OleInitialize, OleUninitialize, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_MOVE,
    DROPEFFECT_SCROLL,
};
use windows::Win32::System::SystemServices::{
    MK_CONTROL, MK_LBUTTON, MK_SHIFT, MODIFIERKEYS_FLAGS,
};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    BHID_DataObject, IDataObjectAsyncCapability, IShellFolder, SHBindToParent,
    SHCreateShellItemArrayFromIDLists, SHParseDisplayName,
};

/// Keep on the worker's STA for its whole lifetime, including message pumping
/// between requests. Never move OLE initialization across threads.
pub struct OleScope(PhantomData<Rc<()>>);

impl OleScope {
    pub fn initialize() -> windows::core::Result<Self> {
        unsafe { OleInitialize(None) }?;
        Ok(Self(PhantomData))
    }
}

impl Drop for OleScope {
    fn drop(&mut self) {
        unsafe { OleUninitialize() };
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferKind {
    Copy,
    Move,
}

impl TransferKind {
    fn effect(self) -> DROPEFFECT {
        match self {
            Self::Copy => DROPEFFECT_COPY,
            Self::Move => DROPEFFECT_MOVE,
        }
    }

    fn keys(self) -> MODIFIERKEYS_FLAGS {
        match self {
            Self::Copy => MK_CONTROL,
            Self::Move => MK_SHIFT,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DispatchOutcome {
    /// Preparation failed before contacting a drop target. Native fallback is safe.
    NotStarted,
    /// The target returned, possibly after cancellation or external delegation.
    Returned,
    /// The target was contacted and failed. Never retry through another engine.
    Failed,
}

pub(crate) fn teracopy_copy_handler_registered() -> bool {
    registration::is_registered()
}

struct Pidl(*mut ITEMIDLIST);

impl Pidl {
    fn parse(path: &Path) -> windows::core::Result<Self> {
        let name = paths::parsing_name(path);
        let mut pidl = Self(std::ptr::null_mut());
        unsafe { SHParseDisplayName(PCWSTR(name.as_ptr()), None, &mut pidl.0, 0, None) }?;
        if pidl.0.is_null() {
            return Err(windows::core::Error::from_hresult(
                windows::Win32::Foundation::E_FAIL,
            ));
        }
        Ok(pidl)
    }
}

impl Drop for Pidl {
    fn drop(&mut self) {
        unsafe { CoTaskMemFree(Some(self.0.cast())) };
    }
}

struct PreparedTransfer {
    target: IDropTarget,
    data: IDataObject,
}

impl PreparedTransfer {
    fn new(paths: &[PathBuf], destination: &Path, owner: HWND) -> windows::core::Result<Self> {
        let sources = paths
            .iter()
            .map(|path| Pidl::parse(path))
            .collect::<Result<Vec<_>, _>>()?;
        let raw: Vec<*const ITEMIDLIST> = sources.iter().map(|pidl| pidl.0.cast_const()).collect();
        unsafe {
            let items = SHCreateShellItemArrayFromIDLists(&raw)?;
            let data: IDataObject = items.BindToHandler(None, &BHID_DataObject)?;
            // Match the validated synchronous handoff. Third-party handlers can
            // still delegate externally; this flag is not a completion promise.
            if let Ok(asynchronous) = data.cast::<IDataObjectAsyncCapability>() {
                asynchronous.SetAsyncMode(false)?;
            }
            let destination = Pidl::parse(destination)?;
            let mut child = std::ptr::null_mut();
            let parent: IShellFolder = SHBindToParent(destination.0, Some(&mut child))?;
            let target = parent.GetUIObjectOf(owner, &[child.cast_const()], None)?;
            Ok(Self { target, data })
        }
    }

    fn dispatch(&self, kind: TransferKind) -> windows::core::Result<()> {
        let point = POINTL { x: 0, y: 0 };
        let mut effect = kind.effect();
        unsafe {
            if let Err(error) =
                self.target
                    .DragEnter(&self.data, MK_LBUTTON | kind.keys(), point, &mut effect)
            {
                let _ = self.target.DragLeave();
                return Err(error);
            }
            if DROPEFFECT(effect.0 & !DROPEFFECT_SCROLL.0) != kind.effect() {
                // A rejected drop is not permission to perform it ourselves.
                let _ = self.target.DragLeave();
                return Ok(());
            }
            effect = kind.effect();
            self.target
                .Drop(&self.data, kind.keys(), point, &mut effect)?;
        }
        // In particular, DROPEFFECT_NONE must not cause a native retry: Shell
        // handlers may cancel OR take responsibility for an optimized move.
        Ok(())
    }
}

/// Caller must run on a message-pumped STA, retain its OLE scope, and pass paths
/// validated by the application's security and archive/namespace policy.
pub fn dispatch(
    paths: &[PathBuf],
    destination: &Path,
    owner: HWND,
    kind: TransferKind,
) -> DispatchOutcome {
    let Ok(_ole) = OleScope::initialize() else {
        return DispatchOutcome::NotStarted;
    };
    if paths.is_empty() {
        return DispatchOutcome::NotStarted;
    }
    let prepared = match PreparedTransfer::new(paths, destination, owner) {
        Ok(prepared) => prepared,
        Err(error) => {
            log::debug!("[ShellTransfer] Preparation unavailable: {}", error.code());
            return DispatchOutcome::NotStarted;
        }
    };
    match prepared.dispatch(kind) {
        Ok(()) => DispatchOutcome::Returned,
        Err(error) => {
            log::warn!(
                "[ShellTransfer] Target call failed; no retry: {}",
                error.code()
            );
            DispatchOutcome::Failed
        }
    }
}
