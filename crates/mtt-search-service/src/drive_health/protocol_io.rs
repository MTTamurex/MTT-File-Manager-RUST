use std::ffi::c_void;

use windows::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, HANDLE};
use windows::Win32::System::Ioctl::IOCTL_STORAGE_QUERY_PROPERTY;
use windows::Win32::System::IO::DeviceIoControl;

use super::windows_io::open_handle;

const PROTOCOL_DESCRIPTOR_SIZE: usize = 48;
const PROTOCOL_DATA_SIZE: usize = 40;
const IOCTL_ATA_PASS_THROUGH: u32 = 0x0004_D02C;
const IOCTL_SCSI_PASS_THROUGH: u32 = 0x0004_D004;

pub(super) struct ProtocolQuery {
    pub(super) property_id: u32,
    pub(super) protocol_type: u32,
    pub(super) data_type: u32,
    pub(super) request_value: u32,
    pub(super) request_subvalue: u32,
    pub(super) data_len: usize,
}

pub(super) fn query_protocol_data(
    handle: HANDLE,
    request: ProtocolQuery,
) -> Result<Vec<u8>, String> {
    let mut buffer = vec![0u8; PROTOCOL_DESCRIPTOR_SIZE + request.data_len];
    write_u32(&mut buffer, 0, request.property_id)?;
    write_u32(&mut buffer, 4, 0)?;
    write_u32(&mut buffer, 8, request.protocol_type)?;
    write_u32(&mut buffer, 12, request.data_type)?;
    write_u32(&mut buffer, 16, request.request_value)?;
    write_u32(&mut buffer, 20, request.request_subvalue)?;
    write_u32(&mut buffer, 24, PROTOCOL_DATA_SIZE as u32)?;
    write_u32(&mut buffer, 28, request.data_len as u32)?;

    let mut returned = 0;
    let buffer_ptr = buffer.as_mut_ptr();
    unsafe {
        DeviceIoControl(
            handle,
            IOCTL_STORAGE_QUERY_PROPERTY,
            Some(buffer_ptr.cast_const().cast::<c_void>()),
            buffer.len() as u32,
            Some(buffer_ptr.cast::<c_void>()),
            buffer.len() as u32,
            Some(&mut returned),
            None,
        )
    }
    .map_err(|error| format!("protocol-specific query failed: {error}"))?;

    let returned = returned as usize;
    if returned < PROTOCOL_DESCRIPTOR_SIZE
        || read_u32(&buffer, 0)? as usize != PROTOCOL_DESCRIPTOR_SIZE
        || read_u32(&buffer, 4)? as usize != PROTOCOL_DESCRIPTOR_SIZE
    {
        return Err("protocol descriptor header is invalid".to_string());
    }
    let data_offset = read_u32(&buffer, 24)? as usize;
    let data_len = read_u32(&buffer, 28)? as usize;
    let start = 8usize
        .checked_add(data_offset)
        .ok_or_else(|| "protocol data offset overflow".to_string())?;
    let end = start
        .checked_add(data_len)
        .ok_or_else(|| "protocol data length overflow".to_string())?;
    if data_offset < PROTOCOL_DATA_SIZE
        || data_len < request.data_len
        || end > returned
        || end > buffer.len()
    {
        return Err("protocol data range is invalid".to_string());
    }
    Ok(buffer[start..start + request.data_len].to_vec())
}

#[repr(C)]
#[derive(Default)]
struct AtaPassThroughEx {
    length: u16,
    ata_flags: u16,
    path_id: u8,
    target_id: u8,
    lun: u8,
    reserved_as_uchar: u8,
    data_transfer_length: u32,
    timeout_value: u32,
    reserved_as_ulong: u32,
    data_buffer_offset: usize,
    previous_task_file: [u8; 8],
    current_task_file: [u8; 8],
}

#[repr(C)]
struct AtaPacket {
    pass_through: AtaPassThroughEx,
    data: [u8; 512],
}

pub(super) fn ata_identify(physical_disk_number: u32) -> Result<[u8; 512], String> {
    ata_command(physical_disk_number, 0xEC, 0)
}

pub(super) fn ata_smart_read(physical_disk_number: u32, feature: u8) -> Result<[u8; 512], String> {
    ata_command(physical_disk_number, 0xB0, feature)
}

fn ata_command(physical_disk_number: u32, command: u8, feature: u8) -> Result<[u8; 512], String> {
    let path = format!(r"\\.\PhysicalDrive{}", physical_disk_number);
    let handle = open_handle(&path, GENERIC_READ.0 | GENERIC_WRITE.0)?;
    let mut packet = AtaPacket {
        pass_through: AtaPassThroughEx::default(),
        data: [0; 512],
    };
    packet.pass_through.length = std::mem::size_of::<AtaPassThroughEx>() as u16;
    packet.pass_through.ata_flags = 0x03;
    packet.pass_through.data_transfer_length = 512;
    packet.pass_through.timeout_value = 10;
    packet.pass_through.data_buffer_offset = std::mem::offset_of!(AtaPacket, data);
    packet.pass_through.current_task_file[0] = feature;
    packet.pass_through.current_task_file[1] = 1;
    if command == 0xB0 {
        packet.pass_through.current_task_file[2] = 1;
        packet.pass_through.current_task_file[3] = 0x4F;
        packet.pass_through.current_task_file[4] = 0xC2;
    }
    packet.pass_through.current_task_file[5] = 0xA0;
    packet.pass_through.current_task_file[6] = command;

    let mut returned = 0;
    let packet_ptr = &mut packet as *mut AtaPacket;
    unsafe {
        DeviceIoControl(
            handle.raw(),
            IOCTL_ATA_PASS_THROUGH,
            Some(packet_ptr.cast_const().cast::<c_void>()),
            std::mem::size_of::<AtaPacket>() as u32,
            Some(packet_ptr.cast::<c_void>()),
            std::mem::size_of::<AtaPacket>() as u32,
            Some(&mut returned),
            None,
        )
    }
    .map_err(|error| format!("ATA pass-through failed: {error}"))?;
    if returned < std::mem::size_of::<AtaPacket>() as u32
        || packet.pass_through.data_transfer_length < 512
        || packet.pass_through.current_task_file[6] & 0x21 != 0
    {
        return Err("ATA command did not return valid data".to_string());
    }
    Ok(packet.data)
}

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
struct SatPacket {
    pass_through: ScsiPassThrough,
    sense: [u8; 32],
    data: [u8; 512],
}

pub(super) fn sat_identify(drive_letter: char) -> Result<[u8; 512], String> {
    sat_command(drive_letter, 0xEC, 0)
}

pub(super) fn sat_smart_read(drive_letter: char, feature: u8) -> Result<[u8; 512], String> {
    sat_command(drive_letter, 0xB0, feature)
}

fn sat_command(drive_letter: char, command: u8, feature: u8) -> Result<[u8; 512], String> {
    let path = format!(r"\\.\{}:", drive_letter);
    let handle = open_handle(&path, GENERIC_READ.0 | GENERIC_WRITE.0)?;
    let mut packet = SatPacket {
        pass_through: ScsiPassThrough::default(),
        sense: [0; 32],
        data: [0; 512],
    };
    packet.pass_through.length = std::mem::size_of::<ScsiPassThrough>() as u16;
    packet.pass_through.cdb_length = 16;
    packet.pass_through.sense_info_length = packet.sense.len() as u8;
    packet.pass_through.data_in = 1;
    packet.pass_through.data_transfer_length = 512;
    packet.pass_through.timeout_value = 10;
    packet.pass_through.data_buffer_offset = std::mem::offset_of!(SatPacket, data);
    packet.pass_through.sense_info_offset = std::mem::offset_of!(SatPacket, sense) as u32;
    packet.pass_through.cdb[0] = 0x85;
    packet.pass_through.cdb[1] = 4 << 1;
    packet.pass_through.cdb[2] = 0x0E;
    packet.pass_through.cdb[4] = feature;
    packet.pass_through.cdb[6] = 1;
    if command == 0xB0 {
        packet.pass_through.cdb[8] = 1;
        packet.pass_through.cdb[10] = 0x4F;
        packet.pass_through.cdb[12] = 0xC2;
    }
    packet.pass_through.cdb[13] = 0xA0;
    packet.pass_through.cdb[14] = command;

    let mut returned = 0;
    let packet_ptr = &mut packet as *mut SatPacket;
    unsafe {
        DeviceIoControl(
            handle.raw(),
            IOCTL_SCSI_PASS_THROUGH,
            Some(packet_ptr.cast_const().cast::<c_void>()),
            std::mem::size_of::<SatPacket>() as u32,
            Some(packet_ptr.cast::<c_void>()),
            std::mem::size_of::<SatPacket>() as u32,
            Some(&mut returned),
            None,
        )
    }
    .map_err(|error| format!("SAT pass-through failed: {error}"))?;
    if returned < std::mem::size_of::<SatPacket>() as u32
        || packet.pass_through.scsi_status != 0
        || packet.pass_through.data_transfer_length < 512
    {
        return Err("SAT command did not return valid data".to_string());
    }
    Ok(packet.data)
}

fn read_u32(buffer: &[u8], offset: usize) -> Result<u32, String> {
    let bytes = buffer
        .get(offset..offset + 4)
        .ok_or_else(|| "protocol descriptor is truncated".to_string())?;
    Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
}

fn write_u32(buffer: &mut [u8], offset: usize, value: u32) -> Result<(), String> {
    let destination = buffer
        .get_mut(offset..offset + 4)
        .ok_or_else(|| "protocol query buffer is truncated".to_string())?;
    destination.copy_from_slice(&value.to_le_bytes());
    Ok(())
}
