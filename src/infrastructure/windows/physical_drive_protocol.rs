//! Native protocol queries used to refine generic storage descriptors.

use std::ffi::c_void;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Ioctl::IOCTL_STORAGE_QUERY_PROPERTY;
use windows::Win32::System::IO::DeviceIoControl;

const STORAGE_ADAPTER_PROTOCOL_SPECIFIC_PROPERTY: u32 = 49;
const STORAGE_DEVICE_PROTOCOL_SPECIFIC_PROPERTY: u32 = 50;
const PROPERTY_STANDARD_QUERY: u32 = 0;
const PROTOCOL_TYPE_ATA: u32 = 2;
const PROTOCOL_TYPE_NVME: u32 = 3;
const PROTOCOL_DATA_TYPE_IDENTIFY: u32 = 1;
const PROTOCOL_SPECIFIC_DATA_SIZE: usize = 40;
const PROTOCOL_DESCRIPTOR_SIZE: usize = 8 + PROTOCOL_SPECIFIC_DATA_SIZE;

pub(super) fn query_nvme_serial(handle: HANDLE, disk_number: u32) -> Option<String> {
    let query = |handle| {
        query_protocol_identify(
            handle,
            STORAGE_ADAPTER_PROTOCOL_SPECIFIC_PROPERTY,
            PROTOCOL_TYPE_NVME,
            4096,
        )
    };
    let identify = query(handle).or_else(|| {
        with_read_write_handle(&format!("\\\\.\\PhysicalDrive{}", disk_number), query)
    })?;

    parse_ascii_field(identify.get(4..24)?)
}

pub(super) fn query_usb_ata_firmware(handle: HANDLE, drive_letter: char) -> Option<String> {
    query_protocol_identify(
        handle,
        STORAGE_DEVICE_PROTOCOL_SPECIFIC_PROPERTY,
        PROTOCOL_TYPE_ATA,
        512,
    )
    .and_then(|identify| parse_ata_identify_string(&identify, 23, 4))
    .or_else(|| query_sat_identify(drive_letter))
}

fn query_protocol_identify(
    handle: HANDLE,
    property_id: u32,
    protocol_type: u32,
    identify_size: usize,
) -> Option<Vec<u8>> {
    let mut buffer = vec![0u8; PROTOCOL_DESCRIPTOR_SIZE + identify_size];
    write_u32(&mut buffer, 0, property_id);
    write_u32(&mut buffer, 4, PROPERTY_STANDARD_QUERY);
    write_u32(&mut buffer, 8, protocol_type);
    write_u32(&mut buffer, 12, PROTOCOL_DATA_TYPE_IDENTIFY);
    write_u32(&mut buffer, 24, PROTOCOL_SPECIFIC_DATA_SIZE as u32);
    write_u32(&mut buffer, 28, identify_size as u32);

    let buffer_ptr = buffer.as_mut_ptr();
    let mut bytes_returned = 0;
    let success = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_STORAGE_QUERY_PROPERTY,
            Some(buffer_ptr.cast::<c_void>()),
            buffer.len() as u32,
            Some(buffer_ptr.cast::<c_void>()),
            buffer.len() as u32,
            Some(&mut bytes_returned),
            None,
        )
    };

    if success.is_err() || bytes_returned < PROTOCOL_DESCRIPTOR_SIZE as u32 {
        return None;
    }

    let returned_len = bytes_returned as usize;
    let version = read_u32(&buffer, 0)? as usize;
    let descriptor_size = read_u32(&buffer, 4)? as usize;
    let data_offset = read_u32(&buffer, 24)? as usize;
    let data_len = read_u32(&buffer, 28)? as usize;
    let data_start = 8usize.checked_add(data_offset)?;
    let data_end = data_start.checked_add(data_len)?;

    if version != PROTOCOL_DESCRIPTOR_SIZE
        || descriptor_size != PROTOCOL_DESCRIPTOR_SIZE
        || data_offset < PROTOCOL_SPECIFIC_DATA_SIZE
        || data_len < identify_size
        || data_end > returned_len
        || data_end > buffer.len()
    {
        return None;
    }

    Some(buffer[data_start..data_start + identify_size].to_vec())
}

fn query_sat_identify(drive_letter: char) -> Option<String> {
    const IOCTL_SCSI_PASS_THROUGH: u32 = 0x0004_D004;
    const SCSI_IOCTL_DATA_IN: u8 = 1;
    const SENSE_SIZE: usize = 32;

    #[repr(C)]
    #[derive(Default)]
    struct ScsiPassThrough {
        length: u16,
        scsi_status: u8,
        path_id: u8,
        target_id: u8,
        lun: u8,
        cdb_length: u8,
        sense_info_length: u8,
        data_in: u8,
        data_transfer_length: u32,
        timeout_value: u32,
        data_buffer_offset: usize,
        sense_info_offset: u32,
        cdb: [u8; 16],
    }

    #[repr(C)]
    struct SatIdentifyPacket {
        pass_through: ScsiPassThrough,
        sense: [u8; SENSE_SIZE],
        identify: [u8; 512],
    }

    with_read_write_handle(&format!("\\\\.\\{}:", drive_letter), |handle| {
        let mut packet = SatIdentifyPacket {
            pass_through: ScsiPassThrough::default(),
            sense: [0; SENSE_SIZE],
            identify: [0; 512],
        };
        packet.pass_through.length = std::mem::size_of::<ScsiPassThrough>() as u16;
        packet.pass_through.cdb_length = 16;
        packet.pass_through.sense_info_length = SENSE_SIZE as u8;
        packet.pass_through.data_in = SCSI_IOCTL_DATA_IN;
        packet.pass_through.data_transfer_length = packet.identify.len() as u32;
        packet.pass_through.timeout_value = 10;
        packet.pass_through.sense_info_offset =
            std::mem::offset_of!(SatIdentifyPacket, sense) as u32;
        packet.pass_through.data_buffer_offset = std::mem::offset_of!(SatIdentifyPacket, identify);

        // SCSI ATA PASS-THROUGH(16), PIO data-in, one sector, IDENTIFY DEVICE.
        packet.pass_through.cdb[0] = 0x85;
        packet.pass_through.cdb[1] = 4 << 1;
        packet.pass_through.cdb[2] = 0x0E;
        packet.pass_through.cdb[6] = 1;
        packet.pass_through.cdb[14] = 0xEC;

        let mut bytes_returned = 0;
        let success = unsafe {
            DeviceIoControl(
                handle,
                IOCTL_SCSI_PASS_THROUGH,
                Some((&packet as *const SatIdentifyPacket).cast::<c_void>()),
                std::mem::size_of::<SatIdentifyPacket>() as u32,
                Some((&mut packet as *mut SatIdentifyPacket).cast::<c_void>()),
                std::mem::size_of::<SatIdentifyPacket>() as u32,
                Some(&mut bytes_returned),
                None,
            )
        };

        if success.is_err()
            || packet.pass_through.scsi_status != 0
            || packet.pass_through.data_transfer_length < 512
        {
            return None;
        }

        parse_ata_identify_string(&packet.identify, 23, 4)
    })
}

fn with_read_write_handle<T>(
    device_path: &str,
    query: impl FnOnce(HANDLE) -> Option<T>,
) -> Option<T> {
    let wide_path: Vec<u16> = device_path
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide_path.as_ptr()),
            GENERIC_READ.0 | GENERIC_WRITE.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )
    };
    let handle = match handle {
        Ok(handle) if handle != INVALID_HANDLE_VALUE => handle,
        _ => return None,
    };

    let result = query(handle);
    let _ = unsafe { CloseHandle(handle) };
    result
}

fn parse_ata_identify_string(data: &[u8], first_word: usize, word_count: usize) -> Option<String> {
    let start = first_word.checked_mul(2)?;
    let end = start.checked_add(word_count.checked_mul(2)?)?;
    let bytes = data.get(start..end)?;
    let mut decoded = Vec::with_capacity(bytes.len());

    for word in bytes.chunks_exact(2) {
        decoded.push(word[1]);
        decoded.push(word[0]);
    }

    parse_ascii_field(&decoded)
}

fn parse_ascii_field(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty()
        || bytes
            .iter()
            .any(|&byte| byte != 0 && !(byte.is_ascii_graphic() || byte == b' '))
    {
        return None;
    }

    let value = String::from_utf8_lossy(bytes)
        .trim_matches(|character| character == '\0' || character == ' ')
        .to_string();
    (!value.is_empty()).then_some(value)
}

fn read_u32(buffer: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        buffer.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn write_u32(buffer: &mut [u8], offset: usize, value: u32) {
    buffer[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::{parse_ascii_field, parse_ata_identify_string};

    #[test]
    fn parses_nvme_serial_field_without_translation() {
        assert_eq!(
            parse_ascii_field(b"50026B77851D736D    "),
            Some("50026B77851D736D".to_string())
        );
    }

    #[test]
    fn parses_word_swapped_ata_firmware() {
        let mut identify = [0u8; 512];
        identify[46..54].copy_from_slice(b"0010    ");

        assert_eq!(
            parse_ata_identify_string(&identify, 23, 4),
            Some("0001".to_string())
        );
    }

    #[test]
    fn rejects_non_ascii_protocol_data() {
        assert_eq!(parse_ascii_field(&[0xFF; 8]), None);
    }
}
