use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime;

pub(super) fn version_two_index(path: &Path, original_size: u64) -> Option<Vec<u8>> {
    let mut original_path: Vec<u16> = path.as_os_str().encode_wide().collect();
    if original_path.starts_with(&[b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16]) {
        original_path.drain(..4);
    }
    // The index must contain a regular absolute drive path, never an UNC,
    // device, relative or traversal path whose restore destination is ambiguous.
    if original_path.len() < 4
        || !matches!(original_path[0], 65..=90 | 97..=122)
        || original_path[1] != b':' as u16
        || !matches!(original_path[2], 47 | 92)
        || original_path.contains(&0)
        || original_path[2..].contains(&(b':' as u16))
        || original_path
            .split(|unit| matches!(*unit, 47 | 92))
            .skip(1)
            .any(|part| part == [46] || part == [46, 46])
    {
        return None;
    }
    for unit in &mut original_path {
        if *unit == b'/' as u16 {
            *unit = b'\\' as u16;
        }
    }
    original_path.push(0);
    let path_length = u32::try_from(original_path.len()).ok()?;

    let file_time = unsafe { GetSystemTimeAsFileTime() };
    let timestamp = ((file_time.dwHighDateTime as u64) << 32) | file_time.dwLowDateTime as u64;

    let mut data = Vec::with_capacity(28 + original_path.len() * 2);
    data.extend_from_slice(&2u64.to_le_bytes());
    data.extend_from_slice(&original_size.to_le_bytes());
    data.extend_from_slice(&timestamp.to_le_bytes());
    data.extend_from_slice(&path_length.to_le_bytes());
    for unit in original_path {
        data.extend_from_slice(&unit.to_le_bytes());
    }
    Some(data)
}
