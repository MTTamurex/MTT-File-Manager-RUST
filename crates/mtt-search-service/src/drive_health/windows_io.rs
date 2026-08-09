use std::ffi::c_void;

use windows::core::{HRESULT, PCWSTR};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_IO_PENDING, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
    WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, GetDriveTypeW, FILE_FLAGS_AND_ATTRIBUTES, FILE_FLAG_OVERLAPPED, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Ioctl::IOCTL_STORAGE_QUERY_PROPERTY;
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows::Win32::System::IO::{CancelIoEx, DeviceIoControl, GetOverlappedResult, OVERLAPPED};

const IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS: u32 = 0x0056_0000;
const STORAGE_DEVICE_PROPERTY: u32 = 0;
const PROPERTY_STANDARD_QUERY: u32 = 0;
const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_FIXED: u32 = 3;
const IOCTL_TIMEOUT_MS: u32 = 5_000;

pub(super) const BUS_TYPE_ATAPI: u32 = 2;
pub(super) const BUS_TYPE_ATA: u32 = 3;
pub(super) const BUS_TYPE_USB: u32 = 7;
pub(super) const BUS_TYPE_SATA: u32 = 11;
pub(super) const BUS_TYPE_NVME: u32 = 17;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct DiskExtent {
    disk_number: u32,
    starting_offset: i64,
    extent_length: i64,
}

#[repr(C)]
struct VolumeDiskExtents {
    number_of_disk_extents: u32,
    extents: [DiskExtent; 16],
}

pub(super) struct Handle(HANDLE);

// Windows kernel handles are process-wide and may be closed from another thread.
unsafe impl Send for Handle {}

impl Handle {
    pub(super) fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

pub(super) struct DeviceContext {
    pub(super) physical_disk_number: u32,
    pub(super) bus_type: u32,
    pub(super) interface: String,
    pub(super) descriptor_model: Option<String>,
    pub(super) descriptor_serial: Option<String>,
    pub(super) descriptor_firmware: Option<String>,
    pub(super) physical_handle: Handle,
}

pub(super) fn open_device(drive_letter: char) -> Result<DeviceContext, String> {
    let root = format!("{}:\\", drive_letter);
    let root_wide = wide(&root);
    let drive_type = unsafe { GetDriveTypeW(PCWSTR(root_wide.as_ptr())) };
    if drive_type != DRIVE_FIXED && drive_type != DRIVE_REMOVABLE {
        return Err("drive is not local fixed or removable storage".to_string());
    }

    let volume_path = format!(r"\\.\{}:", drive_letter);
    let volume = open_handle(&volume_path, 0)?;
    let disk_number = query_single_extent(volume.raw())?;
    drop(volume);

    let physical_path = format!(r"\\.\PhysicalDrive{}", disk_number);
    let physical_handle = open_handle(&physical_path, GENERIC_READ.0 | GENERIC_WRITE.0)?;
    let descriptor = query_device_descriptor(physical_handle.raw())?;
    if matches!(descriptor.bus_type, 14..=16) {
        return Err("virtual storage is not supported".to_string());
    }

    Ok(DeviceContext {
        physical_disk_number: disk_number,
        bus_type: descriptor.bus_type,
        interface: bus_type_name(descriptor.bus_type).to_string(),
        descriptor_model: descriptor.model,
        descriptor_serial: descriptor.serial,
        descriptor_firmware: descriptor.firmware,
        physical_handle,
    })
}

pub(super) fn open_handle(path: &str, access: u32) -> Result<Handle, String> {
    open_handle_with_flags(path, access, FILE_FLAG_OVERLAPPED)
}

pub(super) fn open_synchronous_handle(path: &str, access: u32) -> Result<Handle, String> {
    open_handle_with_flags(path, access, FILE_FLAGS_AND_ATTRIBUTES(0))
}

fn open_handle_with_flags(
    path: &str,
    access: u32,
    flags: FILE_FLAGS_AND_ATTRIBUTES,
) -> Result<Handle, String> {
    let path_wide = wide(path);
    let handle = unsafe {
        CreateFileW(
            PCWSTR(path_wide.as_ptr()),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            flags,
            None,
        )
    }
    .map_err(|error| format!("open device failed: {error}"))?;
    if handle == INVALID_HANDLE_VALUE {
        return Err("open device returned an invalid handle".to_string());
    }
    Ok(Handle(handle))
}

/// Runs an IOCTL on an overlapped handle and drains cancellation before the
/// caller-owned buffers or `OVERLAPPED` can leave scope. A non-cooperating
/// driver can stall that drain, so callers run inside the single globally
/// bounded health worker; its outer timeout returns the IPC handler while this
/// frame safely retains the operation's memory and handles.
pub(super) unsafe fn device_io_control(
    handle: HANDLE,
    control_code: u32,
    input: Option<*const c_void>,
    input_len: u32,
    output: Option<*mut c_void>,
    output_len: u32,
    operation: &str,
) -> Result<u32, String> {
    let event = Handle(
        CreateEventW(None, true, false, None)
            .map_err(|error| format!("{operation} event creation failed: {error}"))?,
    );
    let mut overlapped = OVERLAPPED {
        hEvent: event.raw(),
        ..Default::default()
    };

    match DeviceIoControl(
        handle,
        control_code,
        input,
        input_len,
        output,
        output_len,
        None,
        Some(&mut overlapped),
    ) {
        Ok(()) => {}
        Err(error) if error.code() == HRESULT::from_win32(ERROR_IO_PENDING.0) => {
            match WaitForSingleObject(event.raw(), IOCTL_TIMEOUT_MS) {
                WAIT_OBJECT_0 => {}
                WAIT_TIMEOUT => {
                    let _ = CancelIoEx(handle, Some(&overlapped));
                    let mut ignored = 0;
                    let _ = GetOverlappedResult(handle, &overlapped, &mut ignored, true);
                    return Err(format!("{operation} timed out"));
                }
                _ => {
                    let _ = CancelIoEx(handle, Some(&overlapped));
                    let mut ignored = 0;
                    let _ = GetOverlappedResult(handle, &overlapped, &mut ignored, true);
                    return Err(format!("{operation} wait failed"));
                }
            }
        }
        Err(error) => return Err(format!("{operation} failed: {error}")),
    }

    let mut returned = 0;
    GetOverlappedResult(handle, &overlapped, &mut returned, false)
        .map_err(|error| format!("{operation} completion failed: {error}"))?;
    Ok(returned)
}

fn query_single_extent(volume: HANDLE) -> Result<u32, String> {
    let mut extents = VolumeDiskExtents {
        number_of_disk_extents: 0,
        extents: [DiskExtent::default(); 16],
    };
    let returned = unsafe {
        device_io_control(
            volume,
            IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS,
            None,
            0,
            Some((&mut extents as *mut VolumeDiskExtents).cast::<c_void>()),
            std::mem::size_of::<VolumeDiskExtents>() as u32,
            "volume extent query",
        )
    }?;

    let required =
        std::mem::offset_of!(VolumeDiskExtents, extents) + std::mem::size_of::<DiskExtent>();
    if returned < required as u32 || extents.number_of_disk_extents != 1 {
        return Err("volume does not map to exactly one physical disk".to_string());
    }
    Ok(extents.extents[0].disk_number)
}

struct DeviceDescriptor {
    bus_type: u32,
    model: Option<String>,
    serial: Option<String>,
    firmware: Option<String>,
}

fn query_device_descriptor(handle: HANDLE) -> Result<DeviceDescriptor, String> {
    let mut query = [0u8; 12];
    query[0..4].copy_from_slice(&STORAGE_DEVICE_PROPERTY.to_le_bytes());
    query[4..8].copy_from_slice(&PROPERTY_STANDARD_QUERY.to_le_bytes());
    let mut output = vec![0u8; 4096];
    let returned = unsafe {
        device_io_control(
            handle,
            IOCTL_STORAGE_QUERY_PROPERTY,
            Some(query.as_ptr().cast::<c_void>()),
            query.len() as u32,
            Some(output.as_mut_ptr().cast::<c_void>()),
            output.len() as u32,
            "storage descriptor query",
        )
    }?;

    let returned = returned as usize;
    if returned < 36 {
        return Err("storage descriptor is truncated".to_string());
    }
    let descriptor_size = read_u32(&output, 4)? as usize;
    if descriptor_size < 36 || descriptor_size > returned {
        return Err("storage descriptor size is invalid".to_string());
    }
    let descriptor = &output[..descriptor_size];
    let vendor = descriptor_string(descriptor, read_u32(&output, 12)?);
    let product = descriptor_string(descriptor, read_u32(&output, 16)?);
    let model = match (vendor, product) {
        (Some(vendor), Some(product)) => Some(format!("{vendor} {product}")),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    };

    Ok(DeviceDescriptor {
        bus_type: read_u32(&output, 28)?,
        model,
        firmware: descriptor_string(descriptor, read_u32(&output, 20)?),
        serial: descriptor_string(descriptor, read_u32(&output, 24)?),
    })
}

fn descriptor_string(buffer: &[u8], offset: u32) -> Option<String> {
    let start = usize::try_from(offset).ok()?;
    if start == 0 || start >= buffer.len() {
        return None;
    }
    let tail = &buffer[start..];
    let end = tail
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(tail.len());
    let value = std::str::from_utf8(&tail[..end]).ok()?.trim();
    if value.is_empty() || value.chars().any(char::is_control) {
        None
    } else {
        Some(value.to_string())
    }
}

fn read_u32(buffer: &[u8], offset: usize) -> Result<u32, String> {
    let bytes = buffer
        .get(offset..offset + 4)
        .ok_or_else(|| "storage descriptor field is truncated".to_string())?;
    Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
}

fn bus_type_name(bus_type: u32) -> &'static str {
    match bus_type {
        2 => "ATAPI",
        3 => "ATA",
        7 => "USB",
        11 => "SATA",
        17 => "NVMe",
        _ => "Unknown",
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}
