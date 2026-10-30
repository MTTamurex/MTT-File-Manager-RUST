//! Windows stream metrics require a complete NTFS file reference, including
//! the sequence number which the index's normalized record keys omit.
use super::{
    read_i64_le, read_u16_le, read_u32_le, read_u64_le, FileMetrics, HandleGuard, FILE_SIGNATURE,
    MFT_RECORD_IN_USE,
};
use windows::Win32::{
    Foundation::HANDLE,
    Storage::FileSystem::{
        FileIdType, FileStreamInfo, GetFileInformationByHandleEx, GetFileSizeEx, OpenFileById,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_NO_RECALL, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_ID_DESCRIPTOR, FILE_ID_DESCRIPTOR_0, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE,
    },
};

#[cfg(test)]
mod tests;

pub(super) fn reference_from_record(frn: u64, record: &[u8]) -> Option<u64> {
    const RECORD_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;
    if frn & !RECORD_MASK != 0
        || record.get(..4)? != FILE_SIGNATURE
        || read_u16_le(record, 0x16)? & MFT_RECORD_IN_USE == 0
        || read_u64_le(record, 0x20)? != 0
    {
        return None;
    }
    let sequence = read_u16_le(record, 0x10)?;
    Some(frn | ((sequence as u64) << 48))
}

pub(super) fn read_by_reference(volume_hint: HANDLE, reference: u64) -> Option<FileMetrics> {
    let descriptor = FILE_ID_DESCRIPTOR {
        dwSize: std::mem::size_of::<FILE_ID_DESCRIPTOR>() as u32,
        Type: FileIdType,
        Anonymous: FILE_ID_DESCRIPTOR_0 {
            FileId: reference as i64,
        },
    };
    let raw = unsafe {
        OpenFileById(
            volume_hint,
            &descriptor,
            FILE_READ_ATTRIBUTES.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_NO_RECALL | FILE_FLAG_OPEN_REPARSE_POINT,
        )
    }
    .ok()?;
    let handle = HandleGuard::new(raw)?;
    let mut logical = 0i64;
    // A failed query is unknown, not a successfully measured empty file.
    unsafe { GetFileSizeEx(handle.as_raw(), &mut logical) }.ok()?;
    if logical < 0 {
        return None;
    }
    const MAX_STREAM_BYTES: usize = 16 * 1024 * 1024;
    let mut words = vec![0u64; 8192];
    loop {
        let buffer_bytes = words.len() * std::mem::size_of::<u64>();
        let result = unsafe {
            GetFileInformationByHandleEx(
                handle.as_raw(),
                FileStreamInfo,
                words.as_mut_ptr() as *mut _,
                buffer_bytes as u32,
            )
        };
        if result.is_ok() {
            let bytes =
                unsafe { std::slice::from_raw_parts(words.as_ptr() as *const u8, buffer_bytes) };
            return parse_streams(bytes, logical as u64);
        }
        let error = result.err()?.code();
        if (error != windows::core::HRESULT::from_win32(122)
            && error != windows::core::HRESULT::from_win32(234))
            || buffer_bytes >= MAX_STREAM_BYTES
        {
            return None;
        }
        words.resize((buffer_bytes * 2).min(MAX_STREAM_BYTES).div_ceil(8), 0);
    }
}

fn parse_streams(bytes: &[u8], expected_logical: u64) -> Option<FileMetrics> {
    let mut offset = 0usize;
    let mut saw_default = false;
    let mut metrics = FileMetrics {
        logical_size: expected_logical,
        allocated_size: 0,
    };
    loop {
        let next = read_u32_le(bytes, offset)? as usize;
        let name_bytes = read_u32_le(bytes, offset + 4)? as usize;
        let stream_size = read_i64_le(bytes, offset + 8)?;
        let allocated = read_i64_le(bytes, offset + 16)?;
        let name_end = offset.checked_add(24)?.checked_add(name_bytes)?;
        if name_bytes == 0
            || !name_bytes.is_multiple_of(2)
            || name_end > bytes.len()
            || stream_size < 0
            || allocated < 0
        {
            return None;
        }
        let name: Vec<_> = bytes[offset + 24..name_end]
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        if String::from_utf16(&name)
            .ok()?
            .eq_ignore_ascii_case("::$DATA")
        {
            if saw_default || stream_size as u64 != expected_logical {
                return None;
            }
            saw_default = true;
            metrics.logical_size = stream_size as u64;
        }
        metrics.allocated_size = metrics.allocated_size.saturating_add(allocated as u64);
        if next == 0 {
            return saw_default.then_some(metrics);
        }
        if next < 24 || !next.is_multiple_of(8) || offset.checked_add(next)? < name_end {
            return None;
        }
        offset = offset.checked_add(next)?;
    }
}
