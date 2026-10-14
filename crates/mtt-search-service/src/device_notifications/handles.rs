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
    CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING,
};

use crate::usn_journal::DiscoveredVolume;
use crate::volume_lifecycle::VolumeLifecycle;

struct Context {
    lifecycle: Arc<VolumeLifecycle>,
    drive_letter: char,
    volume_id: String,
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
        self.registrations.retain(|id, _| {
            volumes
                .iter()
                .any(|volume| volume.volume_id.eq_ignore_ascii_case(id))
        });
    }

    pub(crate) fn ensure(
        &mut self,
        volume: &DiscoveredVolume,
        lifecycle: &Arc<VolumeLifecycle>,
    ) -> Result<(), String> {
        let key = volume.volume_id.to_ascii_lowercase();
        if let Some(registration) = self.registrations.get(&key) {
            if registration
                .context
                .as_ref()
                .is_some_and(|context| !context.rearm.load(Ordering::Acquire))
            {
                return Ok(());
            }
        }
        self.registrations.remove(&key);
        let path: Vec<u16> = volume
            .volume_id
            .trim_end_matches('\\')
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let handle = unsafe {
            CreateFileW(
                PCWSTR(path.as_ptr()),
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
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
        Ok(())
    }
}

unsafe extern "system" fn callback(
    _notification: HCMNOTIFICATION,
    context: *const c_void,
    action: CM_NOTIFY_ACTION,
    _event: *const CM_NOTIFY_EVENT_DATA,
    _size: u32,
) -> u32 {
    let context = unsafe { &*context.cast::<Context>() };
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        match action {
            CM_NOTIFY_ACTION_DEVICEQUERYREMOVE => {
                eprintln!("[PNP] Query-remove received for {}:", context.drive_letter);
                context.close();
                if !context.lifecycle.prepare_remove(&context.volume_id) {
                    context.rearm.store(true, Ordering::Release);
                    context.lifecycle.device_arrived();
                    return ERROR_CANCELLED.0;
                }
                eprintln!(
                    "[PNP] Service handles released for {}:",
                    context.drive_letter
                );
            }
            CM_NOTIFY_ACTION_DEVICEQUERYREMOVEFAILED => {
                context.rearm.store(true, Ordering::Release);
                context.lifecycle.removal_failed(&context.volume_id);
            }
            CM_NOTIFY_ACTION_DEVICEREMOVEPENDING | CM_NOTIFY_ACTION_DEVICEREMOVECOMPLETE => {
                context.close();
                context.lifecycle.disconnect_by_id(&context.volume_id);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indexing_progress::IndexingProgress;
    use crate::volume_indices;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn handle_query_remove_closes_registration_handle_and_stops_indexer() {
        let (wake, _events) = mpsc::channel();
        let lifecycle = VolumeLifecycle::new(
            Arc::new(AtomicBool::new(false)),
            volume_indices::new_shared(),
            Arc::new(IndexingProgress::new()),
            wake,
        );
        let volume = DiscoveredVolume {
            drive_letter: 'R',
            volume_id: r"\\?\Volume{abcdef01-2345-6789-abcd-ef0123456789}\".to_string(),
            label: String::new(),
            file_system: "exFAT".to_string(),
            usn_supported: false,
        };
        let ticket = lifecycle.claim_worker(&volume).unwrap();
        let stop = ticket.stop_signal();
        let worker = std::thread::spawn(move || {
            while !stop.is_stopped() {
                stop.wait_or_stop(Duration::from_millis(10));
            }
            drop(ticket);
        });
        let handle = unsafe {
            windows::Win32::System::Threading::CreateEventW(None, true, false, None).unwrap()
        };
        let context = Context {
            lifecycle: lifecycle.clone(),
            drive_letter: volume.drive_letter,
            volume_id: volume.volume_id.clone(),
            handle: Mutex::new(Some(handle.0 as usize)),
            rearm: AtomicBool::new(false),
        };
        let result = unsafe {
            callback(
                HCMNOTIFICATION::default(),
                (&context as *const Context).cast(),
                CM_NOTIFY_ACTION_DEVICEQUERYREMOVE,
                std::ptr::null(),
                0,
            )
        };
        assert_eq!(result, ERROR_SUCCESS.0);
        assert!(context.handle.lock().is_none());
        assert!(lifecycle.acquire_volume('R').is_err());
        worker.join().unwrap();

        let result = unsafe {
            callback(
                HCMNOTIFICATION::default(),
                (&context as *const Context).cast(),
                CM_NOTIFY_ACTION_DEVICEQUERYREMOVEFAILED,
                std::ptr::null(),
                0,
            )
        };
        assert_eq!(result, ERROR_SUCCESS.0);
        assert!(context.rearm.load(Ordering::Acquire));
        assert!(lifecycle.acquire_volume('R').is_ok());
    }
}
