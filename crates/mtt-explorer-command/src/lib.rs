#![cfg(target_os = "windows")]

mod class_factory;
mod explorer_command;
mod registry;

use std::sync::atomic::AtomicU32;

use windows::core::{GUID, HRESULT};

pub use class_factory::{DllCanUnloadNow, DllGetClassObject};

pub(crate) const CLSID_MTT_EXPLORER_COMMAND: GUID =
    GUID::from_u128(0x9f2265b8_8c7a_4b65_93a7_fae34ccb0347);
pub(crate) const S_OK: HRESULT = HRESULT(0);
pub(crate) const S_FALSE: HRESULT = HRESULT(1);
pub(crate) const E_POINTER_HR: HRESULT = HRESULT(0x80004003u32 as i32);
pub(crate) const E_NOINTERFACE_HR: HRESULT = HRESULT(0x80004002u32 as i32);
pub(crate) const E_NOTIMPL_HR: HRESULT = HRESULT(0x80004001u32 as i32);
pub(crate) const E_FAIL_HR: HRESULT = HRESULT(0x80004005u32 as i32);
pub(crate) const E_OUTOFMEMORY_HR: HRESULT = HRESULT(0x8007000eu32 as i32);
pub(crate) const E_INVALIDARG_HR: HRESULT = HRESULT(0x80070057u32 as i32);
pub(crate) const CLASS_E_NOAGGREGATION_HR: HRESULT = HRESULT(0x80040110u32 as i32);
pub(crate) const CLASS_E_CLASSNOTAVAILABLE_HR: HRESULT = HRESULT(0x80040111u32 as i32);

pub(crate) static LIVE_OBJECTS: AtomicU32 = AtomicU32::new(0);
pub(crate) static SERVER_LOCKS: AtomicU32 = AtomicU32::new(0);
