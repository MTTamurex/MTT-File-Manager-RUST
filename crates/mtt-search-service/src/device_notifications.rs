use std::ffi::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Register_Notification, CM_Unregister_Notification, CM_NOTIFY_ACTION,
    CM_NOTIFY_ACTION_DEVICEINTERFACEARRIVAL, CM_NOTIFY_ACTION_DEVICEINTERFACEREMOVAL,
    CM_NOTIFY_EVENT_DATA, CM_NOTIFY_FILTER, CM_NOTIFY_FILTER_0, CM_NOTIFY_FILTER_0_0,
    CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE, CR_SUCCESS, HCMNOTIFICATION,
};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Ioctl::GUID_DEVINTERFACE_VOLUME;

use crate::volume_lifecycle::VolumeLifecycle;

mod handles;
pub(crate) use handles::VolumeHandleNotifications;

pub(crate) struct DeviceNotificationRegistration {
    notification: HCMNOTIFICATION,
    context: *const c_void,
}

pub(crate) fn register_volume_notifications(
    lifecycle: Arc<VolumeLifecycle>,
) -> Result<DeviceNotificationRegistration, String> {
    let filter = CM_NOTIFY_FILTER {
        cbSize: std::mem::size_of::<CM_NOTIFY_FILTER>() as u32,
        FilterType: CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE,
        u: CM_NOTIFY_FILTER_0 {
            DeviceInterface: CM_NOTIFY_FILTER_0_0 {
                ClassGuid: GUID_DEVINTERFACE_VOLUME,
            },
        },
        ..Default::default()
    };

    let context = Arc::into_raw(lifecycle) as *const c_void;
    let mut notification = HCMNOTIFICATION::default();
    let result = unsafe {
        CM_Register_Notification(
            &filter,
            Some(context),
            Some(device_notification_callback),
            &mut notification,
        )
    };
    if result != CR_SUCCESS {
        unsafe {
            drop(Arc::from_raw(context.cast::<VolumeLifecycle>()));
        }
        return Err(format!(
            "CM_Register_Notification failed with configuration code {}",
            result.0
        ));
    }

    Ok(DeviceNotificationRegistration {
        notification,
        context,
    })
}

impl Drop for DeviceNotificationRegistration {
    fn drop(&mut self) {
        let result = unsafe { CM_Unregister_Notification(self.notification) };
        if result != CR_SUCCESS {
            eprintln!(
                "[PNP] CM_Unregister_Notification failed with configuration code {}",
                result.0
            );
            // Keep callback context alive if Windows did not unregister it.
            return;
        }
        unsafe {
            drop(Arc::from_raw(self.context.cast::<VolumeLifecycle>()));
        }
    }
}

unsafe extern "system" fn device_notification_callback(
    _notification: HCMNOTIFICATION,
    context: *const c_void,
    action: CM_NOTIFY_ACTION,
    _event_data: *const CM_NOTIFY_EVENT_DATA,
    _event_data_size: u32,
) -> u32 {
    let Some(context) = (unsafe { context.cast::<VolumeLifecycle>().as_ref() }) else {
        return ERROR_SUCCESS.0;
    };

    match catch_unwind(AssertUnwindSafe(|| handle_device_event(context, action))) {
        Ok(result) => result,
        Err(_) => {
            eprintln!("[PNP] Panic while processing volume device notification");
            ERROR_SUCCESS.0
        }
    }
}

fn handle_device_event(lifecycle: &VolumeLifecycle, action: CM_NOTIFY_ACTION) -> u32 {
    match action {
        CM_NOTIFY_ACTION_DEVICEINTERFACEARRIVAL | CM_NOTIFY_ACTION_DEVICEINTERFACEREMOVAL => {
            lifecycle.wake_discovery();
            ERROR_SUCCESS.0
        }
        _ => ERROR_SUCCESS.0,
    }
}
