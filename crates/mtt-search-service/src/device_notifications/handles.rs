//! Handle notifications are required for query-remove; interface notifications
//! only report arrival/removal. Registrations outlive indexer threads so their
//! callbacks can wait for thread exit without an unregister/join deadlock.
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use windows::core::PCWSTR;
use windows::Win32::Devices::DeviceAndDriverInstallation::*;
use windows::Win32::Foundation::{CloseHandle, ERROR_CANCELLED, ERROR_SUCCESS, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};

use crate::usn_journal::DiscoveredVolume;
use crate::volume_lifecycle::VolumeLifecycle;

mod custom_events;

struct Context {
    lifecycle: Arc<VolumeLifecycle>,
    drive_letter: char,
    volume_id: String,
    volume_serial: u32,
    handle: Mutex<Option<usize>>,
    rearm: AtomicBool,
}

impl Context {
    fn close(&self) {
        if let Some(handle) = self.handle.lock().take() {
            unsafe {
                let _ = CloseHandle(HANDLE(handle as *mut c_void));
            }
        }
    }

    fn is_open(&self) -> bool {
        self.handle.lock().is_some()
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        self.close();
    }
}

struct Registration {
    notification: HCMNOTIFICATION,
    context: Option<Box<Context>>,
}

impl Drop for Registration {
    fn drop(&mut self) {
        // Never executed by the callback or the indexer thread.
        let result = unsafe { CM_Unregister_Notification(self.notification) };
        if let Some(context) = self.context.take() {
            if result == CR_SUCCESS {
                drop(context);
                return;
            }
            // Windows may still call this context. Retain it rather than free
            // callback memory after a failed unregister.
            context.close();
            Box::leak(context);
        }
    }
}

#[derive(Default)]
pub(crate) struct VolumeHandleNotifications {
    registrations: HashMap<String, Registration>,
}

impl VolumeHandleNotifications {
    pub(crate) fn reconcile(&mut self, volumes: &[DiscoveredVolume]) {
        self.registrations.retain(|id, registration| {
            registration.context.as_ref().is_some_and(|context| {
                context
                    .lifecycle
                    .is_removing(&context.volume_id, context.volume_serial)
            }) || volumes
                .iter()
                .any(|volume| registration_key(&volume.volume_id, volume.volume_serial) == *id)
        });
    }

    pub(crate) fn ensure(
        &mut self,
        volume: &DiscoveredVolume,
        lifecycle: &Arc<VolumeLifecycle>,
    ) -> Result<(), String> {
        let key = registration_key(&volume.volume_id, volume.volume_serial);
        if let Some(registration) = self.registrations.get(&key) {
            if registration
                .context
                .as_ref()
                .is_some_and(|context| !context.rearm.load(Ordering::Acquire) && context.is_open())
            {
                return Ok(());
            }
        }
        if lifecycle.is_removing(&volume.volume_id, volume.volume_serial) {
            return Ok(());
        }
        self.registrations.remove(&key);
        let device_path = format!("\\\\.\\{}:", volume.drive_letter);
        let path: Vec<u16> = device_path.encode_utf16().chain(Some(0)).collect();
        let handle = unsafe {
            CreateFileW(
                PCWSTR(path.as_ptr()),
                0x80000000, // GENERIC_READ on the raw volume device
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                None,
            )
        }
        .map_err(|error| format!("Open PnP volume handle failed: {error}"))?;
        let context = Box::new(Context {
            lifecycle: lifecycle.clone(),
            drive_letter: volume.drive_letter,
            volume_id: volume.volume_id.clone(),
            volume_serial: volume.volume_serial,
            handle: Mutex::new(Some(handle.0 as usize)),
            rearm: AtomicBool::new(false),
        });
        let filter = CM_NOTIFY_FILTER {
            cbSize: std::mem::size_of::<CM_NOTIFY_FILTER>() as u32,
            FilterType: CM_NOTIFY_FILTER_TYPE_DEVICEHANDLE,
            u: CM_NOTIFY_FILTER_0 {
                DeviceHandle: CM_NOTIFY_FILTER_0_1 { hTarget: handle },
            },
            ..Default::default()
        };
        let mut notification = HCMNOTIFICATION::default();
        let result = unsafe {
            CM_Register_Notification(
                &filter,
                Some((&*context as *const Context).cast()),
                Some(callback),
                &mut notification,
            )
        };
        if result != CR_SUCCESS {
            return Err(format!(
                "Register PnP handle notification failed: {}",
                result.0
            ));
        }
        self.registrations.insert(
            key,
            Registration {
                notification,
                context: Some(context),
            },
        );
        eprintln!(
            "[PNP] Handle notification registered for {}:",
            volume.drive_letter
        );
        Ok(())
    }
}

unsafe extern "system" fn callback(
    _notification: HCMNOTIFICATION,
    context: *const c_void,
    action: CM_NOTIFY_ACTION,
    event: *const CM_NOTIFY_EVENT_DATA,
    event_size: u32,
) -> u32 {
    let context = unsafe { &*context.cast::<Context>() };
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        match action {
            CM_NOTIFY_ACTION_DEVICEQUERYREMOVE => {
                eprintln!("[PNP] Query-remove received for {}:", context.drive_letter);
                context.close();
                let released = context
                    .lifecycle
                    .prepare_remove_identity(&context.volume_id, context.volume_serial);
                if !released {
                    context.rearm.store(true, Ordering::Release);
                    context.lifecycle.wake_discovery();
                    return ERROR_CANCELLED.0;
                }
                eprintln!(
                    "[PNP] Service handles released for {}:",
                    context.drive_letter
                );
            }
            CM_NOTIFY_ACTION_DEVICEQUERYREMOVEFAILED => {
                context.rearm.store(true, Ordering::Release);
                context
                    .lifecycle
                    .removal_failed_identity(&context.volume_id, context.volume_serial);
            }
            CM_NOTIFY_ACTION_DEVICECUSTOMEVENT => {
                custom_events::handle(context, event, event_size);
            }
            CM_NOTIFY_ACTION_DEVICEREMOVEPENDING | CM_NOTIFY_ACTION_DEVICEREMOVECOMPLETE => {
                context.close();
                context
                    .lifecycle
                    .disconnect_by_identity(&context.volume_id, context.volume_serial);
            }
            _ => {}
        }
        ERROR_SUCCESS.0
    }))
    .unwrap_or(if action == CM_NOTIFY_ACTION_DEVICEQUERYREMOVE {
        ERROR_CANCELLED.0
    } else {
        ERROR_SUCCESS.0
    })
}

fn registration_key(volume_id: &str, volume_serial: u32) -> String {
    format!("{}#{volume_serial:08x}", volume_id.to_ascii_lowercase())
}

#[cfg(test)]
mod tests;
