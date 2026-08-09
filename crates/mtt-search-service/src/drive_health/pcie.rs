use std::ffi::c_void;

use windows::core::{GUID, PCWSTR};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Get_DevNode_PropertyW, CM_Get_Parent, SetupDiDestroyDeviceInfoList,
    SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW, SetupDiGetDeviceInterfaceDetailW,
    CR_SUCCESS, DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, HDEVINFO, SP_DEVICE_INTERFACE_DATA,
    SP_DEVICE_INTERFACE_DETAIL_DATA_W, SP_DEVINFO_DATA,
};
use windows::Win32::Devices::Properties::{DEVPROPTYPE, DEVPROP_TYPE_BYTE, DEVPROP_TYPE_UINT32};
use windows::Win32::Foundation::DEVPROPKEY;
use windows::Win32::System::Ioctl::{
    GUID_DEVINTERFACE_DISK, IOCTL_STORAGE_GET_DEVICE_NUMBER, STORAGE_DEVICE_NUMBER,
};

use super::windows_io::{device_io_control, open_handle};

const MAX_DISK_INTERFACES: u32 = 1024;
const MAX_INTERFACE_DETAIL_BYTES: u32 = 64 * 1024;
const MAX_PARENT_DEPTH: usize = 16;
const PCI_LINK_PROPERTY_FMTID: GUID = GUID::from_u128(0x3ab22e31_8264_4b4e_9af5_a8d2d8e33e62);

pub(super) struct LinkModes {
    pub(super) current: Option<String>,
    pub(super) maximum: Option<String>,
}

struct DeviceInfoSet(HDEVINFO);

impl Drop for DeviceInfoSet {
    fn drop(&mut self) {
        unsafe {
            let _ = SetupDiDestroyDeviceInfoList(self.0);
        }
    }
}

pub(super) fn query(disk_number: u32) -> Option<LinkModes> {
    let device_info_set = DeviceInfoSet(
        unsafe {
            SetupDiGetClassDevsW(
                Some(&GUID_DEVINTERFACE_DISK),
                PCWSTR::null(),
                None,
                DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
            )
        }
        .ok()?,
    );

    for index in 0..MAX_DISK_INTERFACES {
        let mut interface_data = SP_DEVICE_INTERFACE_DATA {
            cbSize: std::mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
            ..Default::default()
        };
        if unsafe {
            SetupDiEnumDeviceInterfaces(
                device_info_set.0,
                None,
                &GUID_DEVINTERFACE_DISK,
                index,
                &mut interface_data,
            )
        }
        .is_err()
        {
            break;
        }

        let Some((device_path, devinst)) = interface_details(device_info_set.0, &interface_data)
        else {
            continue;
        };
        if interface_disk_number(&device_path) == Some(disk_number) {
            return query_ancestors(devinst);
        }
    }
    None
}

fn interface_details(
    device_info_set: HDEVINFO,
    interface_data: &SP_DEVICE_INTERFACE_DATA,
) -> Option<(String, u32)> {
    let mut required_size = 0;
    let _ = unsafe {
        SetupDiGetDeviceInterfaceDetailW(
            device_info_set,
            interface_data,
            None,
            0,
            Some(&mut required_size),
            None,
        )
    };
    if required_size < std::mem::size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32
        || required_size > MAX_INTERFACE_DETAIL_BYTES
    {
        return None;
    }

    let word_size = std::mem::size_of::<usize>();
    let storage_words = (required_size as usize).checked_add(word_size - 1)? / word_size;
    let mut storage = vec![0usize; storage_words];
    let detail = storage
        .as_mut_ptr()
        .cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
    unsafe {
        (*detail).cbSize = std::mem::size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
    }
    let mut device_info = SP_DEVINFO_DATA {
        cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32,
        ..Default::default()
    };
    unsafe {
        SetupDiGetDeviceInterfaceDetailW(
            device_info_set,
            interface_data,
            Some(detail),
            required_size,
            None,
            Some(&mut device_info),
        )
    }
    .ok()?;

    let path_offset = std::mem::offset_of!(SP_DEVICE_INTERFACE_DETAIL_DATA_W, DevicePath);
    let path_units = (required_size as usize).checked_sub(path_offset)? / 2;
    let path_ptr = unsafe { std::ptr::addr_of!((*detail).DevicePath).cast::<u16>() };
    let path_buffer = unsafe { std::slice::from_raw_parts(path_ptr, path_units) };
    let path_len = path_buffer.iter().position(|unit| *unit == 0)?;
    let path = String::from_utf16(&path_buffer[..path_len]).ok()?;
    Some((path, device_info.DevInst))
}

fn interface_disk_number(device_path: &str) -> Option<u32> {
    let handle = open_handle(device_path, 0).ok()?;
    let mut device_number = STORAGE_DEVICE_NUMBER::default();
    let returned = unsafe {
        device_io_control(
            handle.raw(),
            IOCTL_STORAGE_GET_DEVICE_NUMBER,
            None,
            0,
            Some((&mut device_number as *mut STORAGE_DEVICE_NUMBER).cast::<c_void>()),
            std::mem::size_of::<STORAGE_DEVICE_NUMBER>() as u32,
            "storage device number query",
        )
    }
    .ok()?;
    (returned >= std::mem::size_of::<STORAGE_DEVICE_NUMBER>() as u32)
        .then_some(device_number.DeviceNumber)
}

fn query_ancestors(mut devinst: u32) -> Option<LinkModes> {
    for _ in 0..MAX_PARENT_DEPTH {
        let current = format_link(property_u32(devinst, 9), property_u32(devinst, 10));
        let maximum = format_link(property_u32(devinst, 11), property_u32(devinst, 12));
        if current.is_some() || maximum.is_some() {
            return Some(LinkModes { current, maximum });
        }

        let mut parent = 0;
        if unsafe { CM_Get_Parent(&mut parent, devinst, 0) } != CR_SUCCESS || parent == devinst {
            break;
        }
        devinst = parent;
    }
    None
}

fn property_u32(devinst: u32, pid: u32) -> Option<u32> {
    let key = DEVPROPKEY {
        fmtid: PCI_LINK_PROPERTY_FMTID,
        pid,
    };
    let mut property_type = DEVPROPTYPE::default();
    let mut data = [0u8; 4];
    let mut data_size = data.len() as u32;
    if unsafe {
        CM_Get_DevNode_PropertyW(
            devinst,
            &key,
            &mut property_type,
            Some(data.as_mut_ptr()),
            &mut data_size,
            0,
        )
    } != CR_SUCCESS
    {
        return None;
    }

    if property_type == DEVPROP_TYPE_UINT32 && data_size == 4 {
        Some(u32::from_le_bytes(data))
    } else if property_type == DEVPROP_TYPE_BYTE && data_size == 1 {
        Some(u32::from(data[0]))
    } else {
        None
    }
}

fn format_link(speed: Option<u32>, width: Option<u32>) -> Option<String> {
    let speed = match speed? {
        1 => "PCIe 1.0",
        2 => "PCIe 2.0",
        3 => "PCIe 3.0",
        4 => "PCIe 4.0",
        5 => "PCIe 5.0",
        6 => "PCIe 6.0",
        _ => return None,
    };
    Some(match width {
        Some(width) if width != 0 => format!("{speed} x{width}"),
        _ => speed.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_all_known_pcie_speed_codes() {
        for (code, expected) in [
            (1, "PCIe 1.0"),
            (2, "PCIe 2.0"),
            (3, "PCIe 3.0"),
            (4, "PCIe 4.0"),
            (5, "PCIe 5.0"),
            (6, "PCIe 6.0"),
        ] {
            assert_eq!(format_link(Some(code), None).as_deref(), Some(expected));
        }
    }

    #[test]
    fn formats_nonzero_link_width() {
        assert_eq!(
            format_link(Some(4), Some(4)).as_deref(),
            Some("PCIe 4.0 x4")
        );
        assert_eq!(format_link(Some(4), Some(0)).as_deref(), Some("PCIe 4.0"));
    }

    #[test]
    fn rejects_missing_or_unknown_speed() {
        assert_eq!(format_link(None, Some(4)), None);
        assert_eq!(format_link(Some(0), Some(4)), None);
        assert_eq!(format_link(Some(7), Some(4)), None);
    }
}
