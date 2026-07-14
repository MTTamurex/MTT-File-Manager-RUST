use std::mem::{size_of, ManuallyDrop};
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;

use windows::core::{implement, Error, Ref, HRESULT};
use windows::Win32::Foundation::{
    GlobalFree, DATA_S_SAMEFORMATETC, DV_E_DVASPECT, DV_E_FORMATETC, DV_E_LINDEX, DV_E_TYMED,
    E_INVALIDARG, E_NOTIMPL, E_POINTER, OLE_E_ADVISENOTSUPPORTED,
};
use windows::Win32::System::Com::{
    IAdviseSink, IDataObject, IDataObject_Impl, IEnumFORMATETC, IEnumSTATDATA, DATADIR_GET,
    DATADIR_SET, DVASPECT_CONTENT, FORMATETC, STGMEDIUM, STGMEDIUM_0, TYMED_HGLOBAL,
};
use windows::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE, GMEM_ZEROINIT,
};
use windows::Win32::System::Ole::CF_HDROP;
use windows::Win32::UI::Shell::{SHCreateStdEnumFmtEtc, DROPFILES};

pub fn create(paths: &[PathBuf]) -> Result<IDataObject, String> {
    if paths.is_empty() {
        return Err("no files were selected".to_string());
    }

    let mut encoded_paths = Vec::with_capacity(paths.len());
    for path in paths {
        if !path.is_absolute() {
            return Err(format!("drag path is not absolute: '{}'", path.display()));
        }
        let encoded = path.as_os_str().encode_wide().collect::<Vec<_>>();
        if encoded.contains(&0) {
            return Err(format!(
                "drag path contains a null character: '{}'",
                path.display()
            ));
        }
        encoded_paths.push(encoded);
    }

    Ok(FileDataObject { encoded_paths }.into())
}

fn hdrop_format() -> FORMATETC {
    FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    }
}

fn validate_hdrop_format(format: *const FORMATETC) -> HRESULT {
    if format.is_null() {
        return E_POINTER;
    }
    let format = unsafe { &*format };
    if format.cfFormat != CF_HDROP.0 {
        DV_E_FORMATETC
    } else if format.dwAspect != DVASPECT_CONTENT.0 {
        DV_E_DVASPECT
    } else if format.lindex != -1 {
        DV_E_LINDEX
    } else if format.tymed & TYMED_HGLOBAL.0 as u32 == 0 {
        DV_E_TYMED
    } else {
        HRESULT(0)
    }
}

#[implement(IDataObject)]
struct FileDataObject {
    encoded_paths: Vec<Vec<u16>>,
}

#[allow(non_snake_case)]
impl IDataObject_Impl for FileDataObject_Impl {
    fn GetData(&self, format: *const FORMATETC) -> windows::core::Result<STGMEDIUM> {
        validate_hdrop_format(format).ok()?;
        create_hdrop_medium(&self.encoded_paths)
    }

    fn GetDataHere(
        &self,
        _format: *const FORMATETC,
        _medium: *mut STGMEDIUM,
    ) -> windows::core::Result<()> {
        Err(Error::from_hresult(E_NOTIMPL))
    }

    fn QueryGetData(&self, format: *const FORMATETC) -> HRESULT {
        validate_hdrop_format(format)
    }

    fn GetCanonicalFormatEtc(
        &self,
        format_in: *const FORMATETC,
        format_out: *mut FORMATETC,
    ) -> HRESULT {
        if format_in.is_null() || format_out.is_null() {
            return E_POINTER;
        }
        unsafe { (*format_out).ptd = std::ptr::null_mut() };
        DATA_S_SAMEFORMATETC
    }

    fn SetData(
        &self,
        _format: *const FORMATETC,
        _medium: *const STGMEDIUM,
        _release: windows::core::BOOL,
    ) -> windows::core::Result<()> {
        Err(Error::from_hresult(E_NOTIMPL))
    }

    fn EnumFormatEtc(&self, direction: u32) -> windows::core::Result<IEnumFORMATETC> {
        if direction == DATADIR_SET.0 as u32 {
            return Err(Error::from_hresult(E_NOTIMPL));
        }
        if direction != DATADIR_GET.0 as u32 {
            return Err(Error::from_hresult(E_INVALIDARG));
        }
        unsafe { SHCreateStdEnumFmtEtc(&[hdrop_format()]) }
    }

    fn DAdvise(
        &self,
        _format: *const FORMATETC,
        _flags: u32,
        _sink: Ref<'_, IAdviseSink>,
    ) -> windows::core::Result<u32> {
        Err(Error::from_hresult(OLE_E_ADVISENOTSUPPORTED))
    }

    fn DUnadvise(&self, _connection: u32) -> windows::core::Result<()> {
        Err(Error::from_hresult(OLE_E_ADVISENOTSUPPORTED))
    }

    fn EnumDAdvise(&self) -> windows::core::Result<IEnumSTATDATA> {
        Err(Error::from_hresult(OLE_E_ADVISENOTSUPPORTED))
    }
}

fn create_hdrop_medium(encoded_paths: &[Vec<u16>]) -> windows::core::Result<STGMEDIUM> {
    let path_units = encoded_paths.iter().try_fold(1usize, |total, path| {
        total.checked_add(path.len().checked_add(1)?)
    });
    let bytes = path_units
        .and_then(|units| units.checked_mul(size_of::<u16>()))
        .and_then(|path_bytes| size_of::<DROPFILES>().checked_add(path_bytes))
        .ok_or_else(|| Error::from_hresult(windows::Win32::Foundation::E_OUTOFMEMORY))?;

    let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, bytes) }?;
    let base = unsafe { GlobalLock(memory) };
    if base.is_null() {
        let error = Error::from_win32();
        unsafe {
            let _ = GlobalFree(Some(memory));
        }
        return Err(error);
    }

    unsafe {
        std::ptr::write_unaligned(
            base.cast::<DROPFILES>(),
            DROPFILES {
                pFiles: size_of::<DROPFILES>() as u32,
                pt: Default::default(),
                fNC: false.into(),
                fWide: true.into(),
            },
        );

        let mut output = base.cast::<u8>().add(size_of::<DROPFILES>()).cast::<u16>();
        for path in encoded_paths {
            std::ptr::copy_nonoverlapping(path.as_ptr(), output, path.len());
            output = output.add(path.len());
            output.write(0);
            output = output.add(1);
        }
        output.write(0);
        let _ = GlobalUnlock(memory);
    }

    Ok(STGMEDIUM {
        tymed: TYMED_HGLOBAL.0 as u32,
        u: STGMEDIUM_0 { hGlobal: memory },
        pUnkForRelease: ManuallyDrop::new(None),
    })
}

#[cfg(test)]
mod tests {
    use super::{create, hdrop_format};
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use std::path::PathBuf;
    use windows::Win32::Foundation::{
        DV_E_DVASPECT, DV_E_FORMATETC, DV_E_LINDEX, DV_E_TYMED, E_INVALIDARG, E_NOTIMPL, E_POINTER,
    };
    use windows::Win32::System::Com::{DATADIR_GET, DATADIR_SET, DVASPECT_CONTENT, FORMATETC};
    use windows::Win32::System::Ole::{ReleaseStgMedium, CF_HDROP};
    use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};

    #[test]
    fn get_data_preserves_mixed_parent_unicode_and_long_paths() {
        let long_path = format!(
            "C:\\Long\\{}\\{}\\{}\\file.bin",
            "x".repeat(100),
            "y".repeat(100),
            "z".repeat(100)
        );
        let expected = vec![
            PathBuf::from(r"C:\Origem\vídeo 01.mp4"),
            PathBuf::from(r"D:\Outra pasta\日本語.txt"),
            PathBuf::from(long_path),
            PathBuf::from(r"\\server\share\folder\network.dat"),
            PathBuf::from(r"\\?\C:\extended\path.txt"),
        ];
        let data = create(&expected).expect("create file data object");

        let mut medium = unsafe { data.GetData(&hdrop_format()) }.expect("read CF_HDROP");
        let hdrop = HDROP(unsafe { medium.u.hGlobal.0 });
        let count = unsafe { DragQueryFileW(hdrop, u32::MAX, None) };
        let mut actual = Vec::with_capacity(count as usize);
        for index in 0..count {
            let len = unsafe { DragQueryFileW(hdrop, index, None) } as usize;
            let mut buffer = vec![0u16; len + 1];
            let copied = unsafe { DragQueryFileW(hdrop, index, Some(&mut buffer)) } as usize;
            actual.push(PathBuf::from(OsString::from_wide(&buffer[..copied])));
        }
        unsafe { ReleaseStgMedium(&mut medium) };

        assert_eq!(actual, expected);
    }

    #[test]
    fn query_get_data_accepts_only_hdrop_hglobal() {
        let data = create(&[PathBuf::from(r"C:\Source\one.txt")]).expect("create data object");
        assert!(unsafe { data.QueryGetData(&hdrop_format()) }.is_ok());

        let mut wrong = hdrop_format();
        wrong.cfFormat = 1;
        assert_eq!(unsafe { data.QueryGetData(&wrong) }, DV_E_FORMATETC);

        wrong = hdrop_format();
        wrong.dwAspect = 0;
        assert_eq!(unsafe { data.QueryGetData(&wrong) }, DV_E_DVASPECT);

        wrong = hdrop_format();
        wrong.lindex = 0;
        assert_eq!(unsafe { data.QueryGetData(&wrong) }, DV_E_LINDEX);

        wrong = hdrop_format();
        wrong.tymed = 0;
        assert_eq!(unsafe { data.QueryGetData(&wrong) }, DV_E_TYMED);
        assert_eq!(unsafe { data.QueryGetData(std::ptr::null()) }, E_POINTER);
    }

    #[test]
    fn enum_format_etc_exposes_hdrop_and_validates_direction() {
        let data = create(&[PathBuf::from(r"C:\Source\one.txt")]).expect("create data object");
        let formats =
            unsafe { data.EnumFormatEtc(DATADIR_GET.0 as u32) }.expect("enumerate GET formats");
        let mut values = [FORMATETC::default()];
        let mut fetched = 0;
        assert!(unsafe { formats.Next(&mut values, Some(&mut fetched)) }.is_ok());
        assert_eq!(fetched, 1);
        assert_eq!(values[0].cfFormat, CF_HDROP.0);
        assert_eq!(values[0].dwAspect, DVASPECT_CONTENT.0);

        assert_eq!(
            unsafe { data.EnumFormatEtc(DATADIR_SET.0 as u32) }
                .unwrap_err()
                .code(),
            E_NOTIMPL
        );
        assert_eq!(
            unsafe { data.EnumFormatEtc(99) }.unwrap_err().code(),
            E_INVALIDARG
        );
    }

    #[test]
    fn canonical_format_rejects_null_pointers() {
        let data = create(&[PathBuf::from(r"C:\Source\one.txt")]).expect("create data object");
        let format = hdrop_format();
        let mut output = FORMATETC::default();

        assert_eq!(
            unsafe { data.GetCanonicalFormatEtc(std::ptr::null(), &mut output) },
            E_POINTER
        );
        assert_eq!(
            unsafe { data.GetCanonicalFormatEtc(&format, std::ptr::null_mut()) },
            E_POINTER
        );
    }

    #[test]
    fn create_rejects_empty_relative_and_embedded_null_paths() {
        assert!(create(&[]).is_err());
        assert!(create(&[PathBuf::from(r"relative\file.txt")]).is_err());

        let with_null = PathBuf::from(OsString::from_wide(&[
            b'C' as u16,
            b':' as u16,
            b'\\' as u16,
            b'a' as u16,
            0,
            b'b' as u16,
        ]));
        assert!(create(&[with_null]).is_err());
    }
}
