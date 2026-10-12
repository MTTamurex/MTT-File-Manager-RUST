use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::Ordering;

use windows::core::{implement, Error, IUnknown, Interface, Ref, BOOL, GUID, HRESULT};
use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl};

use crate::explorer_command::OpenInMttCommand;
use windows::Win32::UI::Shell::IExplorerCommand;

use crate::{
    CLASS_E_CLASSNOTAVAILABLE_HR, CLASS_E_NOAGGREGATION_HR, CLSID_MTT_EXPLORER_COMMAND,
    E_NOINTERFACE_HR, E_POINTER_HR, LIVE_OBJECTS, SERVER_LOCKS, S_FALSE, S_OK,
};

#[implement(IClassFactory)]
struct ClassFactory;

impl ClassFactory {
    fn new() -> Self {
        LIVE_OBJECTS.fetch_add(1, Ordering::SeqCst);
        Self
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
        if object.is_null() || riid.is_null() {
            return Err(Error::from_hresult(E_POINTER_HR));
        }
        unsafe { *object = ptr::null_mut() };
        if !outer.is_null() {
            return Err(Error::from_hresult(CLASS_E_NOAGGREGATION_HR));
        }
        let requested_iid = unsafe { *riid };
        if requested_iid == IExplorerCommand::IID {
            let command: IExplorerCommand = OpenInMttCommand::new().into();
            unsafe { *object = command.as_raw() };
            std::mem::forget(command);
        } else if requested_iid == IUnknown::IID {
            let command: IExplorerCommand = OpenInMttCommand::new().into();
            let unknown: IUnknown = command.cast()?;
            unsafe { *object = unknown.as_raw() };
            std::mem::forget(unknown);
        } else {
            return Err(Error::from_hresult(E_NOINTERFACE_HR));
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
    if unsafe { *class_id } != CLSID_MTT_EXPLORER_COMMAND {
        return CLASS_E_CLASSNOTAVAILABLE_HR;
    }
    let requested_iid = unsafe { *interface_id };
    if requested_iid != IClassFactory::IID && requested_iid != IUnknown::IID {
        return E_NOINTERFACE_HR;
    }
    let factory: IClassFactory = ClassFactory::new().into();
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
    use super::CLSID_MTT_EXPLORER_COMMAND;

    #[test]
    fn explorer_command_clsid_is_stable() {
        assert_eq!(
            CLSID_MTT_EXPLORER_COMMAND,
            windows::core::GUID::from_u128(0x9f2265b8_8c7a_4b65_93a7_fae34ccb0347)
        );
    }
}
