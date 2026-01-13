use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::core::PCWSTR;
use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
use windows::Win32::UI::Shell::PropertiesSystem::{
    IPropertyStore, SHGetPropertyStoreFromParsingName, GETPROPERTYSTOREFLAGS, GPS_BESTEFFORT,
    GPS_OPENSLOWITEM, GPS_READWRITE,
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
    SHGetPropertyStoreFromParsingName(
        PCWSTR(wide_path.as_ptr()),
        None,
        GETPROPERTYSTOREFLAGS(GPS_READWRITE.0 | GPS_OPENSLOWITEM.0 | GPS_BESTEFFORT.0),
    )
}

// Helper to read property value as u32
pub unsafe fn read_u32(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<u32> {
    let pv = store
        .GetValue(&windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY {
            fmtid: key.fmtid,
            pid: key.pid,
        })
        .ok()?;

    let raw = pv.as_raw();
    let vt = raw.Anonymous.Anonymous.vt;

    match vt {
        VT_UI4 => Some(raw.Anonymous.Anonymous.Anonymous.ulVal),
        VT_I4 => Some(raw.Anonymous.Anonymous.Anonymous.lVal as u32),
        VT_UI2 => Some(raw.Anonymous.Anonymous.Anonymous.uiVal as u32),
        VT_I2 => Some(raw.Anonymous.Anonymous.Anonymous.iVal as u32),
        VT_EMPTY => None,
        other => {
            eprintln!("    [DEBUG] Unexpected VT type for u32: {}", other);
            None
        }
    }
}

// Helper to read property value as u64
pub unsafe fn read_u64(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<u64> {
    let pv = store
        .GetValue(&windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY {
            fmtid: key.fmtid,
            pid: key.pid,
        })
        .ok()?;

    let raw = pv.as_raw();
    let vt = raw.Anonymous.Anonymous.vt;

    match vt {
        VT_UI8 => Some(raw.Anonymous.Anonymous.Anonymous.uhVal as u64),
        VT_I8 => Some(raw.Anonymous.Anonymous.Anonymous.hVal as u64),
        VT_UI4 => Some(raw.Anonymous.Anonymous.Anonymous.ulVal as u64),
        VT_I4 => Some(raw.Anonymous.Anonymous.Anonymous.lVal as u64),
        _ => None,
    }
}

// Helper to read property value as f64 (double)
pub unsafe fn read_f64(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<f64> {
    let pv = store
        .GetValue(&windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY {
            fmtid: key.fmtid,
            pid: key.pid,
        })
        .ok()?;

    let raw = pv.as_raw();
    let vt = raw.Anonymous.Anonymous.vt;

    match vt {
        VT_R8 => Some(raw.Anonymous.Anonymous.Anonymous.dblVal),
        VT_UI4 => Some(raw.Anonymous.Anonymous.Anonymous.ulVal as f64),
        VT_I4 => Some(raw.Anonymous.Anonymous.Anonymous.lVal as f64),
        _ => {
            eprintln!("    [DEBUG] Unexpected VT type for f64: {}", vt);
            None
        }
    }
}

pub unsafe fn read_string(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<String> {
    let pv = match store.GetValue(&windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY {
        fmtid: key.fmtid,
        pid: key.pid,
    }) {
        Ok(v) => v,
        Err(_) => return None,
    };

    let raw = pv.as_raw();
    let vt = raw.Anonymous.Anonymous.vt;

    match vt {
        VT_EMPTY => None,
        VT_LPWSTR => {
            let ptr = raw.Anonymous.Anonymous.Anonymous.pwszVal;
            if !ptr.is_null() {
                let len = (0..).take_while(|&i| *ptr.add(i) != 0).count();
                let slice = std::slice::from_raw_parts(ptr, len);
                Some(String::from_utf16_lossy(slice))
            } else {
                None
            }
        }
        VT_BSTR => {
            let ptr = raw.Anonymous.Anonymous.Anonymous.bstrVal;
            if !ptr.is_null() {
                let len = (0..).take_while(|&i| *ptr.add(i) != 0).count();
                let slice = std::slice::from_raw_parts(ptr, len);
                Some(String::from_utf16_lossy(slice))
            } else {
                None
            }
        }
        VT_VECTOR_LPWSTR => {
            let c_elems = raw.Anonymous.Anonymous.Anonymous.calpwstr.cElems;
            let p_elems = raw.Anonymous.Anonymous.Anonymous.calpwstr.pElems;
            if c_elems > 0 && !p_elems.is_null() {
                let mut result = String::new();
                for i in 0..c_elems {
                    let ptr = *p_elems.add(i as usize);
                    if !ptr.is_null() {
                        let len = (0..).take_while(|&j| *ptr.add(j) != 0).count();
                        let slice = std::slice::from_raw_parts(ptr, len);
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
        .GetValue(&windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY {
            fmtid: key.fmtid,
            pid: key.pid,
        })
        .ok()?;

    let raw = pv.as_raw();
    let vt = raw.Anonymous.Anonymous.vt;

    match vt {
        VT_UI4 => {
            let fourcc = raw.Anonymous.Anonymous.Anonymous.ulVal;
            let bytes = [
                (fourcc & 0xFF) as u8,
                ((fourcc >> 8) & 0xFF) as u8,
                ((fourcc >> 16) & 0xFF) as u8,
                ((fourcc >> 24) & 0xFF) as u8,
            ];
            let codec_str = String::from_utf8(bytes.to_vec()).ok()?;
            if codec_str.trim().is_empty() {
                None
            } else {
                Some(codec_str)
            }
        }
        VT_LPWSTR => {
            let ptr = raw.Anonymous.Anonymous.Anonymous.pwszVal;
            if !ptr.is_null() {
                let len = (0..).take_while(|&i| *ptr.add(i) != 0).count();
                let slice = std::slice::from_raw_parts(ptr, len);
                Some(String::from_utf16_lossy(slice))
            } else {
                None
            }
        }
        _ => None,
    }
}
