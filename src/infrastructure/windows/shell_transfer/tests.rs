use super::*;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::sync::{Arc, Mutex};
use windows::core::{implement, Ref, BOOL, HRESULT};
use windows::Win32::Foundation::{E_FAIL, E_NOTIMPL};
use windows::Win32::System::Com::{
    IAdviseSink, IDataObject_Impl, IEnumFORMATETC, IEnumSTATDATA, FORMATETC, STGMEDIUM,
};
use windows::Win32::System::Ole::{IDropTarget_Impl, DROPEFFECT_NONE};

#[derive(Clone, Copy, Debug)]
enum Behavior {
    Accept,
    AcceptWithScroll,
    WrongEffect,
    CombinedEffect,
    Reject,
    EnterError,
    DropError,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Call {
    Enter { keys: u32, effect: u32 },
    Leave,
    Drop { keys: u32, effect: u32 },
}

#[implement(IDropTarget, IDataObject)]
struct Target {
    behavior: Behavior,
    calls: Arc<Mutex<Vec<Call>>>,
}

#[allow(non_snake_case)]
impl IDropTarget_Impl for Target_Impl {
    fn DragEnter(
        &self,
        _data: Ref<'_, IDataObject>,
        keys: MODIFIERKEYS_FLAGS,
        _point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        self.calls.lock().unwrap().push(Call::Enter {
            keys: keys.0,
            effect: unsafe { (*effect).0 },
        });
        match self.behavior {
            Behavior::EnterError => Err(windows::core::Error::from_hresult(E_FAIL)),
            Behavior::Reject => {
                unsafe { *effect = DROPEFFECT_NONE };
                Ok(())
            }
            Behavior::AcceptWithScroll => {
                unsafe { *effect |= DROPEFFECT_SCROLL };
                Ok(())
            }
            Behavior::WrongEffect => {
                unsafe {
                    *effect = if *effect == DROPEFFECT_COPY {
                        DROPEFFECT_MOVE
                    } else {
                        DROPEFFECT_COPY
                    };
                }
                Ok(())
            }
            Behavior::CombinedEffect => {
                unsafe { *effect = DROPEFFECT_COPY | DROPEFFECT_MOVE };
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn DragOver(
        &self,
        _keys: MODIFIERKEYS_FLAGS,
        _point: &POINTL,
        _effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        Err(windows::core::Error::from_hresult(E_NOTIMPL))
    }

    fn DragLeave(&self) -> windows::core::Result<()> {
        self.calls.lock().unwrap().push(Call::Leave);
        Ok(())
    }

    fn Drop(
        &self,
        _data: Ref<'_, IDataObject>,
        keys: MODIFIERKEYS_FLAGS,
        _point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        self.calls.lock().unwrap().push(Call::Drop {
            keys: keys.0,
            effect: unsafe { (*effect).0 },
        });
        // A target may delegate or optimize a move and return NONE. It must
        // still count as a single handoff, not permission for a second engine.
        unsafe { *effect = DROPEFFECT_NONE };
        if matches!(self.behavior, Behavior::DropError) {
            Err(windows::core::Error::from_hresult(E_FAIL))
        } else {
            Ok(())
        }
    }
}

#[allow(non_snake_case)]
impl IDataObject_Impl for Target_Impl {
    fn GetData(&self, _format: *const FORMATETC) -> windows::core::Result<STGMEDIUM> {
        Err(windows::core::Error::from_hresult(E_NOTIMPL))
    }
    fn GetDataHere(
        &self,
        _format: *const FORMATETC,
        _medium: *mut STGMEDIUM,
    ) -> windows::core::Result<()> {
        Err(windows::core::Error::from_hresult(E_NOTIMPL))
    }
    fn QueryGetData(&self, _format: *const FORMATETC) -> HRESULT {
        E_NOTIMPL
    }
    fn GetCanonicalFormatEtc(&self, _input: *const FORMATETC, _output: *mut FORMATETC) -> HRESULT {
        E_NOTIMPL
    }
    fn SetData(
        &self,
        _format: *const FORMATETC,
        _medium: *const STGMEDIUM,
        _release: BOOL,
    ) -> windows::core::Result<()> {
        Err(windows::core::Error::from_hresult(E_NOTIMPL))
    }
    fn EnumFormatEtc(&self, _direction: u32) -> windows::core::Result<IEnumFORMATETC> {
        Err(windows::core::Error::from_hresult(E_NOTIMPL))
    }
    fn DAdvise(
        &self,
        _format: *const FORMATETC,
        _flags: u32,
        _sink: Ref<'_, IAdviseSink>,
    ) -> windows::core::Result<u32> {
        Err(windows::core::Error::from_hresult(E_NOTIMPL))
    }
    fn DUnadvise(&self, _connection: u32) -> windows::core::Result<()> {
        Err(windows::core::Error::from_hresult(E_NOTIMPL))
    }
    fn EnumDAdvise(&self) -> windows::core::Result<IEnumSTATDATA> {
        Err(windows::core::Error::from_hresult(E_NOTIMPL))
    }
}

fn prepared(behavior: Behavior) -> (PreparedTransfer, Arc<Mutex<Vec<Call>>>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let target: IDropTarget = Target {
        behavior,
        calls: calls.clone(),
    }
    .into();
    let data = target.cast::<IDataObject>().unwrap();
    (PreparedTransfer { target, data }, calls)
}

#[test]
fn shell_copy_and_move_request_only_their_intended_effect() {
    for (kind, effect, modifier) in [
        (TransferKind::Copy, DROPEFFECT_COPY, MK_CONTROL),
        (TransferKind::Move, DROPEFFECT_MOVE, MK_SHIFT),
    ] {
        let (transfer, calls) = prepared(Behavior::Accept);
        transfer.dispatch(kind).unwrap();
        assert_eq!(
            *calls.lock().unwrap(),
            vec![
                Call::Enter {
                    keys: (MK_LBUTTON | modifier).0,
                    effect: effect.0
                },
                Call::Drop {
                    keys: modifier.0,
                    effect: effect.0
                },
            ]
        );
    }
}

#[test]
fn shell_rejected_drop_leaves_without_transferring() {
    let (transfer, calls) = prepared(Behavior::Reject);
    transfer.dispatch(TransferKind::Move).unwrap();
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1], Call::Leave);
}

#[test]
fn shell_scroll_feedback_does_not_reject_an_accepted_transfer() {
    for kind in [TransferKind::Copy, TransferKind::Move] {
        let (transfer, calls) = prepared(Behavior::AcceptWithScroll);
        transfer.dispatch(kind).unwrap();
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert!(matches!(calls[1], Call::Drop { effect, .. } if effect == kind.effect().0));
    }
}

#[test]
fn shell_does_not_accept_a_substituted_or_combined_transfer_effect() {
    for kind in [TransferKind::Copy, TransferKind::Move] {
        for behavior in [Behavior::WrongEffect, Behavior::CombinedEffect] {
            let (transfer, calls) = prepared(behavior);
            transfer.dispatch(kind).unwrap();
            assert_eq!(calls.lock().unwrap()[1], Call::Leave);
        }
    }
}

#[test]
fn shell_target_errors_are_not_retried() {
    for behavior in [Behavior::EnterError, Behavior::DropError] {
        let (transfer, calls) = prepared(behavior);
        assert!(transfer.dispatch(TransferKind::Move).is_err());
        assert_eq!(calls.lock().unwrap().len(), 2);
    }
}

#[test]
fn shell_parsing_converts_verbatim_disk_and_unc_prefixes() {
    for (input, expected) in [
        (r"\\?\C:\Source\日本語.txt", r"C:\Source\日本語.txt"),
        (r"\\?\UNC\server\share\file.txt", r"\\server\share\file.txt"),
        (r"C:\Source\file.txt", r"C:\Source\file.txt"),
    ] {
        assert_eq!(
            paths::parsing_name(Path::new(input)),
            expected.encode_utf16().chain(Some(0)).collect::<Vec<_>>()
        );
    }
}

#[test]
fn shell_parsing_preserves_unpaired_utf16() {
    let mut units: Vec<u16> = r"\\?\C:\Source\".encode_utf16().collect();
    units.push(0xD800);
    let path = OsString::from_wide(&units);
    assert_eq!(
        paths::parsing_name(Path::new(&path)),
        units[4..]
            .iter()
            .copied()
            .chain(Some(0))
            .collect::<Vec<_>>()
    );
}

#[test]
fn shell_resolves_canonical_existing_file_and_folder_without_transfer() {
    let _ole = OleScope::initialize().unwrap();
    let path = std::env::current_exe().unwrap().canonicalize().unwrap();
    assert!(Pidl::parse(&path).is_ok());
    assert!(Pidl::parse(path.parent().unwrap()).is_ok());
}
