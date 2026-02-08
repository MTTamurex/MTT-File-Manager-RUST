use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::core::PCWSTR;
use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
use windows::Win32::UI::Shell::PropertiesSystem::{
    IPropertyStore, SHGetPropertyStoreFromParsingName, GPS_BESTEFFORT,
};

use super::property_keys::*;

pub struct ComGuard {
    initialized: bool,
}

impl ComGuard {
    pub fn new() -> Option<Self> {
        // SAFETY: CoInitializeEx/CoUninitialize balance via RAII; RPC_E_CHANGED_MODE means COM already initialized.
        unsafe {
            let hr = CoInitializeEx(None, COINIT_MULTITHREADED);
            if hr == RPC_E_CHANGED_MODE {
                return Some(Self { initialized: false });
            }
            if hr.is_err() {
                return None;
            }
            Some(Self { initialized: true })
        }
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.initialized {
            unsafe {
                CoUninitialize();
            }
        }
    }
}

pub unsafe fn open_property_store(path: &Path) -> Result<IPropertyStore, windows::core::Error> {
    let wide_path: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    // SAFETY: wide_path is a null-terminated UTF-16 buffer that stays alive for the call.
    // CRITICAL: Use GPS_BESTEFFORT only — NOT GPS_OPENSLOWITEM.
    // GPS_OPENSLOWITEM tells Windows to WAIT for slow/network items (OneDrive cloud files),
    // which can block the calling thread for 30-60+ seconds on cloud-only files.
    // GPS_BESTEFFORT returns whatever metadata is locally cached without blocking.
    SHGetPropertyStoreFromParsingName(PCWSTR(wide_path.as_ptr()), None, GPS_BESTEFFORT)
}

// Helper to read property value as u32
pub unsafe fn read_u32(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<u32> {
    let pv = store
        .GetValue(&PROPERTYKEY {
            fmtid: key.fmtid,
            pid: key.pid,
        } as *const _ as *const _)
        .ok()?;

    let raw = unsafe {
        &*(&pv.Anonymous.Anonymous as *const _
            as *const windows::Win32::System::Com::StructuredStorage::PROPVARIANT_0_0)
    };
    let vt = raw.vt;

    match vt.0 {
        VT_UI4 => Some(unsafe { raw.Anonymous.ulVal }),
        VT_I4 => Some(unsafe { raw.Anonymous.lVal as u32 }),
        VT_UI2 => Some(unsafe { raw.Anonymous.uiVal as u32 }),
        VT_I2 => Some(unsafe { raw.Anonymous.iVal as u32 }),
        VT_LPWSTR | VT_BSTR => {
            // Fallback: Try parsing string as number
            let s = read_string(store, key)?;
            s.chars()
                .filter(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse::<u32>()
                .ok()
        }
        VT_EMPTY => None,
        _ => None,
    }
}

// Helper to read property value as u64
pub unsafe fn read_u64(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<u64> {
    let pv = store
        .GetValue(&PROPERTYKEY {
            fmtid: key.fmtid,
            pid: key.pid,
        } as *const _ as *const _)
        .ok()?;

    let raw = unsafe {
        &*(&pv.Anonymous.Anonymous as *const _
            as *const windows::Win32::System::Com::StructuredStorage::PROPVARIANT_0_0)
    };
    let vt = raw.vt;

    match vt.0 {
        VT_UI8 => Some(unsafe { raw.Anonymous.uhVal }),
        VT_I8 => Some(unsafe { raw.Anonymous.hVal as u64 }),
        VT_UI4 => Some(unsafe { raw.Anonymous.ulVal as u64 }),
        VT_I4 => Some(unsafe { raw.Anonymous.lVal as u64 }),
        VT_LPWSTR | VT_BSTR => {
            let s = read_string(store, key)?;
            s.chars()
                .filter(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse::<u64>()
                .ok()
        }
        _ => None,
    }
}

// Helper to read property value as f64 (double)
pub unsafe fn read_f64(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<f64> {
    let pv = store
        .GetValue(&PROPERTYKEY {
            fmtid: key.fmtid,
            pid: key.pid,
        } as *const _ as *const _)
        .ok()?;

    let raw = unsafe {
        &*(&pv.Anonymous.Anonymous as *const _
            as *const windows::Win32::System::Com::StructuredStorage::PROPVARIANT_0_0)
    };
    let vt = raw.vt;

    match vt.0 {
        VT_R8 => Some(unsafe { raw.Anonymous.dblVal }),
        VT_UI4 => Some(unsafe { raw.Anonymous.ulVal as f64 }),
        VT_I4 => Some(unsafe { raw.Anonymous.lVal as f64 }),
        _ => {
            eprintln!("    [DEBUG] Unexpected VT type for f64: {:?}", vt);
            None
        }
    }
}

pub unsafe fn read_string(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<String> {
    let pv = match store.GetValue(&PROPERTYKEY {
        fmtid: key.fmtid,
        pid: key.pid,
    } as *const _ as *const _)
    {
        Ok(v) => v,
        Err(_) => return None,
    };

    let raw = unsafe {
        &*(&pv.Anonymous.Anonymous as *const _
            as *const windows::Win32::System::Com::StructuredStorage::PROPVARIANT_0_0)
    };
    let vt = raw.vt;

    match vt.0 {
        VT_EMPTY => None,
        VT_LPWSTR => {
            let ptr = unsafe { raw.Anonymous.pwszVal };
            if !ptr.is_null() {
                let len = (0..).take_while(|&i| unsafe { *ptr.0.add(i) != 0 }).count();
                let slice = unsafe { std::slice::from_raw_parts(ptr.0, len) };
                Some(String::from_utf16_lossy(slice))
            } else {
                None
            }
        }
        VT_BSTR => {
            let ptr = unsafe { &raw.Anonymous.bstrVal };
            if !ptr.is_empty() {
                Some(ptr.to_string())
            } else {
                None
            }
        }
        VT_VECTOR_LPWSTR => {
            let c_elems = unsafe { raw.Anonymous.calpwstr.cElems };
            let p_elems = unsafe { raw.Anonymous.calpwstr.pElems };
            if c_elems > 0 && !p_elems.is_null() {
                let mut result = String::new();
                for i in 0..c_elems {
                    let ptr = unsafe { *p_elems.add(i as usize) };
                    if !ptr.is_null() {
                        let len = (0..).take_while(|&j| unsafe { *ptr.0.add(j) != 0 }).count();
                        let slice = unsafe { std::slice::from_raw_parts(ptr.0, len) };
                        let s = String::from_utf16_lossy(slice);
                        if !result.is_empty() {
                            result.push_str(", ");
                        }
                        result.push_str(&s);
                    }
                }
                if result.is_empty() {
                    None
                } else {
                    Some(result)
                }
            } else {
                None
            }
        }
        other => {
            if other != 0 {
                eprintln!(
                    "[DEBUG] read_string: unexpected VT type {} for PKEY {{pid={}}}",
                    other, key.pid
                );
            }
            None
        }
    }
}

// Helper to read FourCC (can be u32 or string)
pub unsafe fn read_fourcc(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<String> {
    let pv = store
        .GetValue(&PROPERTYKEY {
            fmtid: key.fmtid,
            pid: key.pid,
        } as *const _ as *const _)
        .ok()?;

    let raw = unsafe {
        &*(&pv.Anonymous.Anonymous as *const _
            as *const windows::Win32::System::Com::StructuredStorage::PROPVARIANT_0_0)
    };
    let vt = raw.vt;

    match vt.0 {
        VT_UI4 => {
            let fourcc = unsafe { raw.Anonymous.ulVal };
            let bytes = [
                (fourcc & 0xFF) as u8,
                ((fourcc >> 8) & 0xFF) as u8,
                ((fourcc >> 16) & 0xFF) as u8,
                ((fourcc >> 24) & 0xFF) as u8,
            ];
            let codec_str = String::from_utf8(bytes.to_vec()).ok()?;
            // Debug log para verificar FourCC do Property Store
            eprintln!(
                "[DEBUG] read_fourcc VT_UI4: fourcc=0x{:08X}, bytes={:?}, codec_str='{}'",
                fourcc, bytes, codec_str
            );
            if codec_str.trim().is_empty() {
                None
            } else {
                Some(codec_str)
            }
        }
        VT_LPWSTR => {
            let ptr = unsafe { raw.Anonymous.pwszVal };
            if !ptr.is_null() {
                let len = (0..).take_while(|&i| unsafe { *ptr.0.add(i) != 0 }).count();
                let slice = unsafe { std::slice::from_raw_parts(ptr.0, len) };
                let result = String::from_utf16_lossy(slice);
                // Debug log para verificar FourCC do Property Store
                eprintln!("[DEBUG] read_fourcc VT_LPWSTR: result='{}'", result);
                Some(result)
            } else {
                None
            }
        }
        _ => None,
    }
}
