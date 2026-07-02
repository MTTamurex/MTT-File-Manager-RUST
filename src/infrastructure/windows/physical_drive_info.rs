//! Physical drive hardware information (model, serial, firmware, bus type).
//!
//! Uses `DeviceIoControl` with zero-access handles — no admin required.
//! Results are cached per drive letter for the entire session (same pattern
//! as `io_priority::detection::DISK_TYPE_CACHE`).

use std::sync::OnceLock;

use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::IOCTL_STORAGE_QUERY_PROPERTY;

use crate::infrastructure::windows::DriveType;

/// Hardware fields to merge into `DriveInfo`.
#[derive(Default)]
pub struct HardwareFields {
    pub model: Option<String>,
    pub serial_number: Option<String>,
    pub firmware_revision: Option<String>,
    pub bus_type: Option<String>,
}

/// Physical drive hardware descriptor.
#[derive(Clone, Debug)]
pub struct PhysicalDriveInfo {
    /// Combined vendor ID + product ID (e.g. "Samsung SSD 970 EVO 500GB").
    pub model: String,
    /// Drive serial number (may be empty on some RAID/HBA controllers).
    pub serial_number: String,
    /// Firmware revision string.
    pub firmware_revision: String,
    /// Bus type as human-readable string (e.g. "NVMe", "SATA", "USB").
    pub bus_type: String,
}

static PHYSICAL_DRIVE_CACHE: OnceLock<Mutex<FxHashMap<char, PhysicalDriveInfo>>> =
    OnceLock::new();

fn get_cache() -> &'static Mutex<FxHashMap<char, PhysicalDriveInfo>> {
    PHYSICAL_DRIVE_CACHE.get_or_init(|| Mutex::new(FxHashMap::default()))
}

/// Invalidate the cached physical drive info for a drive letter.
/// Call on device arrival/removal to force a re-query on next access.
pub fn invalidate_physical_drive_cache(drive_letter: char) {
    get_cache().lock().remove(&drive_letter.to_ascii_uppercase());
}

/// Query physical drive hardware info for a given drive letter.
///
/// Returns `None` if any step fails (access denied, RAID controller, etc.).
/// Results are cached — subsequent calls are a hashmap lookup.
pub fn query_physical_drive_info(drive_letter: char) -> Option<PhysicalDriveInfo> {
    let letter = drive_letter.to_ascii_uppercase();

    {
        let cache = get_cache().lock();
        if let Some(info) = cache.get(&letter) {
            return Some(info.clone());
        }
    }

    let disk_number = get_physical_disk_number(letter)?;
    let info = query_storage_device_property(disk_number)?;

    get_cache().lock().insert(letter, info.clone());
    Some(info)
}

/// Convenience wrapper for background threads: takes a drive path and
/// `DriveType`, returns hardware fields or all-`None` if not applicable.
///
/// Skips:
/// - `Remote`, `Cdrom`, `RamDisk`, `Unknown` — no physical backing
/// - Virtual drives (Cryptomator, Dokan, WinFsp) — FUSE-backed, no physical
///   device behind them; IOCTL calls would fail or produce meaningless data
pub fn query_hardware_fields(path: &str, drive_type: DriveType) -> HardwareFields {
    if !matches!(drive_type, DriveType::Fixed | DriveType::Removable) {
        return HardwareFields::default();
    }

    let Some(letter) = path.chars().next().map(|c| c.to_ascii_uppercase()) else {
        return HardwareFields::default();
    };

    // Skip virtual drives (Cryptomator, Dokan, WinFsp, etc.) — they report
    // as Fixed but have no physical device behind them.
    if is_virtual_drive(letter) {
        return HardwareFields::default();
    }

    match query_physical_drive_info(letter) {
        Some(hw) => HardwareFields {
            model: Some(hw.model),
            serial_number: Some(hw.serial_number),
            firmware_revision: Some(hw.firmware_revision),
            bus_type: Some(hw.bus_type),
        },
        None => HardwareFields::default(),
    }
}

/// Checks whether a drive letter belongs to a virtual drive (Cryptomator,
/// Dokan, WinFsp) or has a user-configured override.
///
/// Mirrors the logic in `io_priority::detection::determine_disk_type`.
fn is_virtual_drive(drive_letter: char) -> bool {
    use crate::infrastructure::virtual_drive_config;

    if virtual_drive_config::get_drive_override(drive_letter).is_some() {
        return true;
    }

    virtual_drive_config::detect_virtual_drive(drive_letter).is_some()
}

/// Maps a logical drive letter to its physical disk number via
/// `IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS`.
fn get_physical_disk_number(drive_letter: char) -> Option<u32> {
    // IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS is not in the windows crate's
    // typed IOCTL constants, so we use the raw code: CTL_CODE(0x56, 0, 0, 0)
    // = 0x00560000
    const IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS: u32 = 0x00560000;

    #[repr(C)]
    #[derive(Default)]
    struct DiskExtent {
        disk_number: u32,
        starting_offset: i64,
        extent_length: i64,
    }

    #[repr(C)]
    #[derive(Default)]
    struct VolumeDiskExtents {
        number_of_disk_extents: u32,
        // We only need the first extent; extra extents are ignored.
        extents: [DiskExtent; 1],
    }

    let device_path = format!("\\\\.\\{}:", drive_letter);
    let wide_path: Vec<u16> = device_path
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let handle = CreateFileW(
            PCWSTR(wide_path.as_ptr()),
            0, // zero access — no admin required
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        );

        let handle = match handle {
            Ok(h) if h != INVALID_HANDLE_VALUE => h,
            _ => return None,
        };

        let mut extents = VolumeDiskExtents::default();
        let mut bytes_returned: u32 = 0;

        let success = DeviceIoControl(
            handle,
            IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS,
            None,
            0,
            Some(&mut extents as *mut _ as *mut std::ffi::c_void),
            std::mem::size_of::<VolumeDiskExtents>() as u32,
            Some(&mut bytes_returned),
            None,
        );

        let _ = CloseHandle(handle);

        if success.is_ok() && extents.number_of_disk_extents > 0 {
            Some(extents.extents[0].disk_number)
        } else {
            None
        }
    }
}

/// Queries `IOCTL_STORAGE_QUERY_PROPERTY` with `StorageDeviceProperty` on
/// `\\.\PhysicalDriveN` and parses the variable-length descriptor.
fn query_storage_device_property(disk_number: u32) -> Option<PhysicalDriveInfo> {
    const STORAGE_DEVICE_PROPERTY: u32 = 0; // PropertyId
    const PROPERTY_STANDARD_QUERY: u32 = 0; // QueryType

    #[repr(C)]
    struct StoragePropertyQuery {
        property_id: u32,
        query_type: u32,
        additional_parameters: [u8; 1],
    }

    // STORAGE_DEVICE_DESCRIPTOR — fixed header; the rest is variable-length
    // string data appended after the struct.
    //
    // Layout verified against windows 0.61.3 crate:
    //   offset 0:  Version (u32)
    //   offset 4:  Size (u32)
    //   offset 8:  DeviceType (u8)
    //   offset 9:  DeviceTypeModifier (u8)
    //   offset 10: RemovableMedia (u8)
    //   offset 11: CommandQueueing (u8)
    //   offset 12: VendorIdOffset (u32)
    //   offset 16: ProductIdOffset (u32)
    //   offset 20: ProductRevisionOffset (u32)
    //   offset 24: SerialNumberOffset (u32)
    //   offset 28: BusType (u32, STORAGE_BUS_TYPE enum)
    #[repr(C)]
    struct StorageDeviceDescriptor {
        version: u32,
        size: u32,
        device_type: u8,
        device_type_modifier: u8,
        removable_media: u8,
        command_queueing: u8,
        vendor_id_offset: u32,
        product_id_offset: u32,
        product_revision_offset: u32,
        serial_number_offset: u32,
        bus_type: u32,
    }

    let device_path = format!("\\\\.\\PhysicalDrive{}", disk_number);
    let wide_path: Vec<u16> = device_path
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let handle = CreateFileW(
            PCWSTR(wide_path.as_ptr()),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        );

        let handle = match handle {
            Ok(h) if h != INVALID_HANDLE_VALUE => h,
            _ => return None,
        };

        let query = StoragePropertyQuery {
            property_id: STORAGE_DEVICE_PROPERTY,
            query_type: PROPERTY_STANDARD_QUERY,
            additional_parameters: [0],
        };

        // Allocate a generous buffer; the descriptor + strings typically
        // fits in ~1KB. We use 4KB to be safe.
        const BUFFER_SIZE: u32 = 4096;
        let mut buffer = vec![0u8; BUFFER_SIZE as usize];
        let mut bytes_returned: u32 = 0;

        let success = DeviceIoControl(
            handle,
            IOCTL_STORAGE_QUERY_PROPERTY,
            Some(&query as *const _ as *const std::ffi::c_void),
            std::mem::size_of::<StoragePropertyQuery>() as u32,
            Some(buffer.as_mut_ptr() as *mut std::ffi::c_void),
            BUFFER_SIZE,
            Some(&mut bytes_returned),
            None,
        );

        let _ = CloseHandle(handle);

        if !success.is_ok() || bytes_returned == 0 {
            return None;
        }

        // Parse the descriptor header from the start of the buffer.
        if (bytes_returned as usize) < std::mem::size_of::<StorageDeviceDescriptor>() {
            return None;
        }

        let descriptor = &*(buffer.as_ptr() as *const StorageDeviceDescriptor);
        let buf_len = bytes_returned as usize;

        let vendor = read_descriptor_string(&buffer, buf_len, descriptor.vendor_id_offset);
        let product = read_descriptor_string(&buffer, buf_len, descriptor.product_id_offset);
        let firmware =
            read_descriptor_string(&buffer, buf_len, descriptor.product_revision_offset);
        let serial =
            read_descriptor_string(&buffer, buf_len, descriptor.serial_number_offset);

        // Combine vendor + product into a single model string.
        let model = match (vendor.as_str().trim(), product.as_str().trim()) {
            ("", p) if !p.is_empty() => p.to_string(),
            (v, "") if !v.is_empty() => v.to_string(),
            (v, p) if !v.is_empty() && !p.is_empty() => format!("{} {}", v, p),
            _ => String::new(),
        };

        Some(PhysicalDriveInfo {
            model,
            serial_number: serial.trim().to_string(),
            firmware_revision: firmware.trim().to_string(),
            bus_type: bus_type_to_string(descriptor.bus_type),
        })
    }
}

/// Reads a NUL-terminated ASCII string from the descriptor buffer at the
/// given offset. Offsets are relative to the start of the
/// `STORAGE_DEVICE_DESCRIPTOR` struct (i.e. the buffer start).
fn read_descriptor_string(buffer: &[u8], buf_len: usize, offset: u32) -> String {
    if offset == 0 {
        return String::new();
    }

    let start = offset as usize;
    if start >= buf_len {
        return String::new();
    }

    let end = buffer[start..buf_len]
        .iter()
        .position(|&b| b == 0)
        .map(|p| start + p)
        .unwrap_or(buf_len);

    String::from_utf8_lossy(&buffer[start..end]).to_string()
}

/// Maps the `STORAGE_BUS_TYPE` integer to a human-readable string.
fn bus_type_to_string(bus_type: u32) -> String {
    match bus_type {
        0 => "Unknown",
        1 => "SCSI",
        2 => "ATAPI",
        3 => "ATA",
        4 => "IEEE 1394",
        5 => "SSA",
        6 => "Fibre",
        7 => "USB",
        8 => "RAID",
        9 => "iSCSI",
        10 => "SAS",
        11 => "SATA",
        12 => "SD",
        13 => "MMC",
        14 => "Virtual",
        15 => "File Backed Virtual",
        16 => "Storage Spaces",
        17 => "NVMe",
        18 => "SCM",
        19 => "UFS",
        _ => "Other",
    }
    .to_string()
}
