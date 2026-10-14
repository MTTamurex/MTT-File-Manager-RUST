use windows::core::GUID;
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_NOTIFY_EVENT_DATA, CM_NOTIFY_EVENT_DATA_0_1, CM_NOTIFY_FILTER_TYPE_DEVICEHANDLE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GUID_IO_VOLUME_DISMOUNT, GUID_IO_VOLUME_DISMOUNT_FAILED, GUID_IO_VOLUME_LOCK,
    GUID_IO_VOLUME_LOCK_FAILED, GUID_IO_VOLUME_MOUNT, GUID_IO_VOLUME_PREPARING_EJECT,
    GUID_IO_VOLUME_UNLOCK,
};

use super::{Context, Ordering};

pub(super) unsafe fn handle(context: &Context, event: *const CM_NOTIFY_EVENT_DATA, size: u32) {
    let event_guid = custom_event_guid(event, size);
    match event_guid {
        Some(GUID_IO_VOLUME_LOCK)
        | Some(GUID_IO_VOLUME_DISMOUNT)
        | Some(GUID_IO_VOLUME_PREPARING_EJECT) => {
            eprintln!(
                "[PNP] Volume lock/dismount request received for {}:",
                context.drive_letter
            );
            context.close();
            let released = context
                .lifecycle
                .prepare_maintenance_identity(&context.volume_id, context.volume_serial);
            if !released {
                eprintln!(
                    "[PNP] {}:\\ could not release all handles before volume operation",
                    context.drive_letter
                );
            } else if event_guid == Some(GUID_IO_VOLUME_DISMOUNT) {
                context
                    .lifecycle
                    .disconnect_by_identity(&context.volume_id, context.volume_serial);
            }
        }
        Some(GUID_IO_VOLUME_LOCK_FAILED)
        | Some(GUID_IO_VOLUME_DISMOUNT_FAILED)
        | Some(GUID_IO_VOLUME_UNLOCK)
        | Some(GUID_IO_VOLUME_MOUNT) => {
            eprintln!(
                "[PNP] Volume lock/dismount failed or completed for {}:",
                context.drive_letter
            );
            context.rearm.store(true, Ordering::Release);
            context
                .lifecycle
                .removal_failed_identity(&context.volume_id, context.volume_serial);
        }
        _ => {}
    }
}

unsafe fn custom_event_guid(event: *const CM_NOTIFY_EVENT_DATA, size: u32) -> Option<GUID> {
    if event.is_null() {
        return None;
    }
    let guid_offset = std::mem::offset_of!(CM_NOTIFY_EVENT_DATA, u)
        + std::mem::offset_of!(CM_NOTIFY_EVENT_DATA_0_1, EventGuid);
    if (size as usize) < guid_offset + std::mem::size_of::<GUID>() {
        return None;
    }
    let filter_type = std::ptr::read_unaligned(event.cast::<i32>());
    if filter_type != CM_NOTIFY_FILTER_TYPE_DEVICEHANDLE.0 {
        return None;
    }
    Some(std::ptr::read_unaligned(
        event.cast::<u8>().add(guid_offset).cast::<GUID>(),
    ))
}

#[cfg(test)]
mod tests {
    use super::custom_event_guid;

    #[test]
    fn custom_event_parser_rejects_short_payload_before_reading_fields() {
        let short = [0u8; 1];
        assert!(unsafe { custom_event_guid(short.as_ptr().cast(), 1) }.is_none());
        assert!(unsafe { custom_event_guid(std::ptr::null(), 128) }.is_none());
    }
}
