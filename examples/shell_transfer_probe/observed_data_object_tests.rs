use super::*;
use std::mem::ManuallyDrop;
use windows::core::Interface;
use windows::Win32::Foundation::{E_NOTIMPL, HGLOBAL};
use windows::Win32::System::Com::{STGMEDIUM_0, TYMED_ISTREAM};
use windows::Win32::System::Memory::{GlobalAlloc, GMEM_MOVEABLE, GMEM_ZEROINIT};
use windows::Win32::System::Ole::ReleaseStgMedium;

#[implement(IDataObject)]
struct TestData;

#[allow(non_snake_case)]
impl IDataObject_Impl for TestData_Impl {
    fn GetData(&self, _format: *const FORMATETC) -> windows::core::Result<STGMEDIUM> {
        Err(Error::from_hresult(E_NOTIMPL))
    }
    fn GetDataHere(
        &self,
        _format: *const FORMATETC,
        _medium: *mut STGMEDIUM,
    ) -> windows::core::Result<()> {
        Err(Error::from_hresult(E_NOTIMPL))
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
        medium: *const STGMEDIUM,
        release: BOOL,
    ) -> windows::core::Result<()> {
        if release.as_bool() {
            // Simulate an inner object consuming the medium immediately.
            let mut medium = unsafe { std::ptr::read(medium) };
            unsafe { ReleaseStgMedium(&mut medium) };
        }
        Ok(())
    }
    fn EnumFormatEtc(&self, _direction: u32) -> windows::core::Result<IEnumFORMATETC> {
        Err(Error::from_hresult(E_NOTIMPL))
    }
    fn DAdvise(
        &self,
        _format: *const FORMATETC,
        _flags: u32,
        _sink: Ref<'_, IAdviseSink>,
    ) -> windows::core::Result<u32> {
        Err(Error::from_hresult(E_NOTIMPL))
    }
    fn DUnadvise(&self, _connection: u32) -> windows::core::Result<()> {
        Err(Error::from_hresult(E_NOTIMPL))
    }
    fn EnumDAdvise(&self) -> windows::core::Result<IEnumSTATDATA> {
        Err(Error::from_hresult(E_NOTIMPL))
    }
}

struct OwnedMedium(STGMEDIUM);

impl OwnedMedium {
    fn new(value: u32) -> Self {
        let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, size_of::<u32>()) }
            .expect("allocate feedback DWORD");
        let pointer = unsafe { GlobalLock(memory) };
        assert!(!pointer.is_null());
        unsafe {
            pointer.cast::<u32>().write_unaligned(value);
            let _ = GlobalUnlock(memory);
        }
        Self(STGMEDIUM {
            tymed: TYMED_HGLOBAL.0 as u32,
            u: STGMEDIUM_0 { hGlobal: memory },
            pUnkForRelease: ManuallyDrop::new(None),
        })
    }
}

impl Drop for OwnedMedium {
    fn drop(&mut self) {
        unsafe { ReleaseStgMedium(&mut self.0) };
    }
}

fn feedback_format(id: u16) -> FORMATETC {
    FORMATETC {
        cfFormat: id,
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
        ..Default::default()
    }
}

#[test]
fn async_protocol_records_start_end_and_result_without_inventing_success() {
    let observation = Observation::new();
    let data = create(TestData.into(), observation.clone()).unwrap();
    let asynchronous = data.cast::<IDataObjectAsyncCapability>().unwrap();
    unsafe {
        assert!(asynchronous.GetAsyncMode().unwrap().as_bool());
        assert!(!asynchronous.InOperation().unwrap().as_bool());
        asynchronous.StartOperation(None).unwrap();
        assert!(asynchronous.InOperation().unwrap().as_bool());
        asynchronous.EndOperation(E_FAIL, None, 0).unwrap();
        assert!(!asynchronous.InOperation().unwrap().as_bool());
        asynchronous.SetAsyncMode(false).unwrap();
        assert!(!asynchronous.GetAsyncMode().unwrap().as_bool());
    }
    assert_eq!(
        observation.signals(),
        vec![
            Signal::AsyncModeQueried,
            Signal::Started,
            Signal::Ended {
                result: E_FAIL,
                effects: 0
            },
            Signal::AsyncMode(false),
            Signal::AsyncModeQueried,
        ]
    );
}

#[test]
fn feedback_is_observed_before_inner_object_consumes_the_medium() {
    let observation = Observation::new();
    let data = create(TestData.into(), observation.clone()).unwrap();
    let formats = FeedbackFormats::register().unwrap();
    let medium = ManuallyDrop::new(OwnedMedium::new(1));
    unsafe {
        data.SetData(&feedback_format(formats.performed), &medium.0, true)
            .unwrap();
    }
    // The inner object already released the handle; do not release it twice.
    assert_eq!(
        observation.signals(),
        vec![Signal::Feedback {
            format: "Performed DropEffect",
            effect: Some(1),
            accepted: true,
        }]
    );
}

#[test]
fn non_transferred_medium_remains_owned_by_the_caller() {
    let observation = Observation::new();
    let data = create(TestData.into(), observation.clone()).unwrap();
    let formats = FeedbackFormats::register().unwrap();
    let format = feedback_format(formats.paste);
    let medium = OwnedMedium::new(0);
    unsafe { data.SetData(&format, &medium.0, false) }.unwrap();
    assert_eq!(unsafe { read_effect(&format, &medium.0) }, Some(0));
    assert_eq!(
        observation.signals(),
        vec![Signal::Feedback {
            format: "Paste Succeeded",
            effect: Some(0),
            accepted: true,
        }]
    );
}

#[test]
fn malformed_feedback_does_not_dereference_a_non_hglobal_union_member() {
    let format = feedback_format(1);
    let mut medium = STGMEDIUM {
        tymed: TYMED_ISTREAM.0 as u32,
        u: STGMEDIUM_0 {
            hGlobal: HGLOBAL(std::ptr::null_mut()),
        },
        pUnkForRelease: ManuallyDrop::new(None),
    };
    assert_eq!(unsafe { read_effect(&format, &medium) }, None);
    medium.tymed = TYMED_HGLOBAL.0 as u32;
    assert_eq!(unsafe { read_effect(&format, &medium) }, None);
    assert_eq!(unsafe { read_effect(std::ptr::null(), &medium) }, None);
    assert_eq!(unsafe { read_effect(&format, std::ptr::null()) }, None);
}

#[test]
fn ordinary_data_requests_keep_the_inner_result() {
    let data = create(TestData.into(), Observation::new()).unwrap();
    assert_eq!(unsafe { data.QueryGetData(std::ptr::null()) }, E_NOTIMPL);
    assert_eq!(
        unsafe { data.GetData(std::ptr::null()) }
            .err()
            .unwrap()
            .code(),
        E_NOTIMPL
    );
}
