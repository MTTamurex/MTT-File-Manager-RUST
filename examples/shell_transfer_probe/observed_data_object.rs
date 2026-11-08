//! Diagnostic wrapper. It exposes the async negotiation protocol explicitly;
//! this is an opt-in experiment, not a transparent replacement for every Shell interface.

use super::observation::{Observation, Signal};
use std::sync::atomic::{AtomicBool, Ordering};
use windows::core::{implement, Error, Ref, BOOL, HRESULT};
use windows::Win32::Foundation::E_FAIL;
use windows::Win32::System::Com::{
    IAdviseSink, IBindCtx, IDataObject, IDataObject_Impl, IEnumFORMATETC, IEnumSTATDATA,
    DVASPECT_CONTENT, FORMATETC, STGMEDIUM, TYMED_HGLOBAL,
};
use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::UI::Shell::{
    IDataObjectAsyncCapability, IDataObjectAsyncCapability_Impl, CFSTR_LOGICALPERFORMEDDROPEFFECT,
    CFSTR_PASTESUCCEEDED, CFSTR_PERFORMEDDROPEFFECT,
};

#[cfg(test)]
#[path = "observed_data_object_tests.rs"]
mod tests;

struct FeedbackFormats {
    performed: u16,
    logical: u16,
    paste: u16,
}

impl FeedbackFormats {
    fn register() -> windows::core::Result<Self> {
        let ids = unsafe {
            [
                RegisterClipboardFormatW(CFSTR_PERFORMEDDROPEFFECT),
                RegisterClipboardFormatW(CFSTR_LOGICALPERFORMEDDROPEFFECT),
                RegisterClipboardFormatW(CFSTR_PASTESUCCEEDED),
            ]
        };
        if ids.contains(&0) {
            return Err(Error::from_hresult(E_FAIL));
        }
        Ok(Self {
            performed: ids[0] as u16,
            logical: ids[1] as u16,
            paste: ids[2] as u16,
        })
    }

    fn name(&self, id: u16) -> Option<&'static str> {
        if id == self.performed {
            Some("Performed DropEffect")
        } else if id == self.logical {
            Some("Logical Performed DropEffect")
        } else if id == self.paste {
            Some("Paste Succeeded")
        } else {
            None
        }
    }
}

pub fn create(inner: IDataObject, observation: Observation) -> windows::core::Result<IDataObject> {
    let data: IDataObject = ObservedDataObject {
        inner,
        observation,
        formats: FeedbackFormats::register()?,
        async_mode: AtomicBool::new(true),
        in_operation: AtomicBool::new(false),
    }
    .into();
    Ok(data)
}

#[implement(IDataObject, IDataObjectAsyncCapability)]
struct ObservedDataObject {
    inner: IDataObject,
    observation: Observation,
    formats: FeedbackFormats,
    async_mode: AtomicBool,
    in_operation: AtomicBool,
}

// Read before forwarding SetData: with fRelease=true the inner object may take
// ownership of the medium, making it invalid for the observer afterwards.
unsafe fn read_effect(format: *const FORMATETC, medium: *const STGMEDIUM) -> Option<u32> {
    let (format, medium) = unsafe { (format.as_ref()?, medium.as_ref()?) };
    if format.dwAspect != DVASPECT_CONTENT.0
        || format.lindex != -1
        || format.tymed & TYMED_HGLOBAL.0 as u32 == 0
        || medium.tymed != TYMED_HGLOBAL.0 as u32
    {
        return None;
    }
    unsafe {
        let memory = medium.u.hGlobal;
        if GlobalSize(memory) < size_of::<u32>() {
            return None;
        }
        let pointer = GlobalLock(memory);
        if pointer.is_null() {
            return None;
        }
        let value = pointer.cast::<u32>().read_unaligned();
        let _ = GlobalUnlock(memory);
        Some(value)
    }
}

#[allow(non_snake_case)]
impl IDataObject_Impl for ObservedDataObject_Impl {
    fn GetData(&self, format: *const FORMATETC) -> windows::core::Result<STGMEDIUM> {
        unsafe { self.inner.GetData(format) }
    }

    fn GetDataHere(
        &self,
        format: *const FORMATETC,
        medium: *mut STGMEDIUM,
    ) -> windows::core::Result<()> {
        unsafe { self.inner.GetDataHere(format, medium) }
    }

    fn QueryGetData(&self, format: *const FORMATETC) -> HRESULT {
        unsafe { self.inner.QueryGetData(format) }
    }

    fn GetCanonicalFormatEtc(&self, input: *const FORMATETC, output: *mut FORMATETC) -> HRESULT {
        unsafe { self.inner.GetCanonicalFormatEtc(input, output) }
    }

    fn SetData(
        &self,
        format: *const FORMATETC,
        medium: *const STGMEDIUM,
        release: BOOL,
    ) -> windows::core::Result<()> {
        let feedback = unsafe { format.as_ref() }
            .and_then(|format| self.formats.name(format.cfFormat))
            .map(|name| (name, unsafe { read_effect(format, medium) }));
        let result = unsafe { self.inner.SetData(format, medium, release.as_bool()) };
        if let Some((format, effect)) = feedback {
            self.observation.record(Signal::Feedback {
                format,
                effect,
                accepted: result.is_ok(),
            });
        }
        result
    }

    fn EnumFormatEtc(&self, direction: u32) -> windows::core::Result<IEnumFORMATETC> {
        unsafe { self.inner.EnumFormatEtc(direction) }
    }

    fn DAdvise(
        &self,
        format: *const FORMATETC,
        flags: u32,
        sink: Ref<'_, IAdviseSink>,
    ) -> windows::core::Result<u32> {
        unsafe { self.inner.DAdvise(format, flags, sink.as_ref()) }
    }

    fn DUnadvise(&self, connection: u32) -> windows::core::Result<()> {
        unsafe { self.inner.DUnadvise(connection) }
    }

    fn EnumDAdvise(&self) -> windows::core::Result<IEnumSTATDATA> {
        unsafe { self.inner.EnumDAdvise() }
    }
}

#[allow(non_snake_case)]
impl IDataObjectAsyncCapability_Impl for ObservedDataObject_Impl {
    fn SetAsyncMode(&self, enabled: BOOL) -> windows::core::Result<()> {
        self.async_mode.store(enabled.as_bool(), Ordering::SeqCst);
        self.observation
            .record(Signal::AsyncMode(enabled.as_bool()));
        Ok(())
    }

    fn GetAsyncMode(&self) -> windows::core::Result<BOOL> {
        self.observation.record(Signal::AsyncModeQueried);
        Ok(self.async_mode.load(Ordering::SeqCst).into())
    }

    fn StartOperation(&self, _reserved: Ref<'_, IBindCtx>) -> windows::core::Result<()> {
        self.in_operation.store(true, Ordering::SeqCst);
        self.observation.record(Signal::Started);
        Ok(())
    }

    fn InOperation(&self) -> windows::core::Result<BOOL> {
        Ok(self.in_operation.load(Ordering::SeqCst).into())
    }

    fn EndOperation(
        &self,
        result: HRESULT,
        _reserved: Ref<'_, IBindCtx>,
        effects: u32,
    ) -> windows::core::Result<()> {
        self.in_operation.store(false, Ordering::SeqCst);
        self.observation.record(Signal::Ended { result, effects });
        Ok(())
    }
}
