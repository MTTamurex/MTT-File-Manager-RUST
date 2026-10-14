use super::lifecycle::{self, Control};
use crate::infrastructure::windows::OwnedHandle;
use std::ffi::c_void;
use std::sync::Arc;
use std::time::{Duration, Instant};
use windows::core::PCWSTR;
use windows::Win32::Devices::DeviceAndDriverInstallation::*;
use windows::Win32::Foundation::{ERROR_CANCELLED, ERROR_SUCCESS};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GUID_IO_VOLUME_DISMOUNT, GUID_IO_VOLUME_DISMOUNT_FAILED, GUID_IO_VOLUME_LOCK,
    GUID_IO_VOLUME_LOCK_FAILED, GUID_IO_VOLUME_MOUNT, GUID_IO_VOLUME_PREPARING_EJECT,
    GUID_IO_VOLUME_UNLOCK,
};

struct Context {
    letter: char,
    control: Arc<Control>,
    epoch: u64,
    removed: std::sync::atomic::AtomicBool,
}
pub(super) struct Registration {
    notification: HCMNOTIFICATION,
    context: Option<Box<Context>>,
}
// Registration handles can be unregistered from the UI/setup thread. The
// callback context stays alive until CM_Unregister_Notification completes.
unsafe impl Send for Registration {}

impl Registration {
    pub fn new(letter: char, control: Arc<Control>) -> Result<Option<Self>, String> {
        let _native = control.native.lock();
        if lifecycle::is_suspended(letter) {
            return Err("Volume monitoring suspended".to_string());
        }
        let path: Vec<u16> = format!(r"\\.\{}:", letter)
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
        .map_err(|error| format!("Open watcher PnP handle failed: {error}"))?;
        let owned = OwnedHandle::new(handle).ok_or("Invalid watcher PnP handle")?;
        if lifecycle::is_suspended(letter) {
            return Err("Volume monitoring suspended".to_string());
        }
        *control.monitor.lock() = Some(owned);
        let context = Box::new(Context {
            letter,
            control: control.clone(),
            epoch: lifecycle::epoch(letter),
            removed: std::sync::atomic::AtomicBool::new(false),
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
            control.monitor.lock().take();
            if result == CR_INVALID_DATA {
                // Session-only filesystem drivers may have no PnP device
                // behind their directory handles. Preserve native monitoring
                // without keeping an unregistrable raw-volume handle open.
                return Ok(None);
            }
            return Err(format!(
                "Register watcher PnP notification failed: {}",
                result.0
            ));
        }
        Ok(Some(Self {
            notification,
            context: Some(context),
        }))
    }

    pub(super) fn can_release(&self) -> bool {
        self.context.as_ref().is_none_or(|context| {
            context.removed.load(std::sync::atomic::Ordering::Acquire)
                || context.epoch != lifecycle::epoch(context.letter)
                || !lifecycle::is_suspended(context.letter)
        })
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        let result = unsafe { CM_Unregister_Notification(self.notification) };
        if let Some(context) = self.context.take() {
            if result != CR_SUCCESS {
                context
                    .control
                    .stop(Instant::now() + Duration::from_secs(4));
                Box::leak(context);
            }
        }
    }
}

unsafe extern "system" fn callback(
    _: HCMNOTIFICATION,
    context: *const c_void,
    action: CM_NOTIFY_ACTION,
    event: *const CM_NOTIFY_EVENT_DATA,
    size: u32,
) -> u32 {
    let context = &*context.cast::<Context>();
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if context.epoch != lifecycle::epoch(context.letter) { return ERROR_SUCCESS.0; }
        let deadline = Instant::now() + Duration::from_secs(4);
        match action {
            CM_NOTIFY_ACTION_DEVICEQUERYREMOVE => {
                log::info!("[APP-PNP] Query-remove for {}:", context.letter);
                if !lifecycle::suspend(context.letter, deadline) {
                    lifecycle::resume(context.letter);
                    crate::infrastructure::windows::device_change::repaint_after_watcher_lifecycle_change();
                    return ERROR_CANCELLED.0;
                }
                log::info!(
                    "[APP-PNP] Directory watchers released for {}:",
                    context.letter
                );
            }
            CM_NOTIFY_ACTION_DEVICEQUERYREMOVEFAILED => {
                log::info!("[APP-PNP] Removal cancelled for {}:", context.letter);
                lifecycle::resume(context.letter);
            }
            CM_NOTIFY_ACTION_DEVICEREMOVEPENDING | CM_NOTIFY_ACTION_DEVICEREMOVECOMPLETE => {
                lifecycle::suspend(context.letter, deadline);
                if action == CM_NOTIFY_ACTION_DEVICEREMOVECOMPLETE {
                    context.removed.store(true, std::sync::atomic::Ordering::Release);
                }
            }
            CM_NOTIFY_ACTION_DEVICECUSTOMEVENT => match event_guid(event, size) {
                Some(GUID_IO_VOLUME_LOCK)
                | Some(GUID_IO_VOLUME_DISMOUNT)
                | Some(GUID_IO_VOLUME_PREPARING_EJECT) => {
                    lifecycle::suspend(context.letter, deadline);
                }
                Some(GUID_IO_VOLUME_LOCK_FAILED)
                | Some(GUID_IO_VOLUME_DISMOUNT_FAILED)
                | Some(GUID_IO_VOLUME_UNLOCK)
                | Some(GUID_IO_VOLUME_MOUNT) => lifecycle::resume(context.letter),
                _ => {}
            },
            _ => {}
        }
        crate::infrastructure::windows::device_change::repaint_after_watcher_lifecycle_change();
        ERROR_SUCCESS.0
    }))
    .unwrap_or(if action == CM_NOTIFY_ACTION_DEVICEQUERYREMOVE {
        ERROR_CANCELLED.0
    } else {
        ERROR_SUCCESS.0
    })
}

#[cfg(test)]
pub(super) fn simulate_notification(
    letter: char,
    control: Arc<Control>,
    action: CM_NOTIFY_ACTION,
    guid: Option<windows::core::GUID>,
) -> u32 {
    let context = Context {
        letter,
        control,
        epoch: lifecycle::epoch(letter),
        removed: std::sync::atomic::AtomicBool::new(false),
    };
    let event = guid.map(|guid| CM_NOTIFY_EVENT_DATA {
        FilterType: CM_NOTIFY_FILTER_TYPE_DEVICEHANDLE,
        u: CM_NOTIFY_EVENT_DATA_0 {
            DeviceHandle: CM_NOTIFY_EVENT_DATA_0_1 {
                EventGuid: guid,
                ..Default::default()
            },
        },
        ..Default::default()
    });
    unsafe {
        callback(
            HCMNOTIFICATION::default(),
            (&context as *const Context).cast(),
            action,
            event
                .as_ref()
                .map_or(std::ptr::null(), |event| event as *const _),
            event
                .as_ref()
                .map_or(0, |_| std::mem::size_of::<CM_NOTIFY_EVENT_DATA>() as u32),
        )
    }
}

unsafe fn event_guid(event: *const CM_NOTIFY_EVENT_DATA, size: u32) -> Option<windows::core::GUID> {
    let offset = std::mem::offset_of!(CM_NOTIFY_EVENT_DATA, u)
        + std::mem::offset_of!(CM_NOTIFY_EVENT_DATA_0_1, EventGuid);
    if event.is_null() || (size as usize) < offset + std::mem::size_of::<windows::core::GUID>() {
        return None;
    }
    if std::ptr::read_unaligned(event.cast::<i32>()) != CM_NOTIFY_FILTER_TYPE_DEVICEHANDLE.0 {
        return None;
    }
    Some(std::ptr::read_unaligned(
        event.cast::<u8>().add(offset).cast(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_event_rejects_truncated_payload() {
        let payload = [0u8; 1];
        assert!(unsafe { event_guid(payload.as_ptr().cast(), 1) }.is_none());
        assert!(unsafe { event_guid(std::ptr::null(), 128) }.is_none());
    }

    #[test]
    fn delayed_old_registration_cannot_suspend_a_returned_volume() {
        lifecycle::resume('X');
        let (tx, rx) = crossbeam_channel::unbounded();
        let handler = Arc::new(std::sync::Mutex::new(|_: notify::Result<notify::Event>| {}));
        let watcher = notify::windows::ReadDirectoryChangesWatcher::create(handler, tx).unwrap();
        let control = Arc::new(Control {
            native: parking_lot::Mutex::new(lifecycle::NativeState::new(watcher, rx)),
            monitor: parking_lot::Mutex::new(None),
        });
        let context = Context {
            letter: 'X',
            control: control.clone(),
            epoch: lifecycle::epoch('X'),
            removed: std::sync::atomic::AtomicBool::new(false),
        };
        lifecycle::suspend('X', Instant::now() + Duration::from_secs(1));
        lifecycle::arrived('X');
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
        assert!(!lifecycle::is_suspended('X'));
        assert!(!control.is_closed());
        assert!(control.stop(Instant::now() + Duration::from_secs(1)));
    }
}
