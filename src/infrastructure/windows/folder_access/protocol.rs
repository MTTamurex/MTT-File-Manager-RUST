use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use super::os_error_code;

const PIPE_PREFIX: &str = r"\\.\pipe\MTT.FolderAccess.";
pub(super) const MAX_PATH_BYTES: usize = 32_767 * 2;

pub(super) struct HelperRequest {
    pub pipe_name: String,
    pub requester_pid: u32,
}

impl HelperRequest {
    pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Self, u32> {
        let mut args = args.into_iter();
        if args.next().as_deref() != Some(OsStr::new("--ipc-v1")) {
            return Err(87);
        }
        let pipe_name = args
            .next()
            .and_then(|arg| arg.into_string().ok())
            .ok_or(87u32)?;
        let pid = args
            .next()
            .and_then(|arg| arg.into_string().ok())
            .ok_or(87u32)?;
        let suffix = pipe_name.strip_prefix(PIPE_PREFIX).ok_or(87u32)?;
        if args.next().is_some()
            || suffix.len() != 32
            || !suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
            || pid.is_empty()
            || !pid.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(87);
        }
        let requester_pid = pid
            .parse::<u32>()
            .ok()
            .filter(|pid| *pid != 0)
            .ok_or(87u32)?;
        Ok(Self {
            pipe_name,
            requester_pid,
        })
    }

    pub fn new() -> Result<Self, u32> {
        let guid = unsafe { windows::Win32::System::Com::CoCreateGuid() }.map_err(os_error_code)?;
        Ok(Self {
            pipe_name: format!(
                "{PIPE_PREFIX}{:08x}{:04x}{:04x}{:016x}",
                guid.data1,
                guid.data2,
                guid.data3,
                u64::from_be_bytes(guid.data4)
            ),
            requester_pid: std::process::id(),
        })
    }
}

pub(super) fn encode_path(path: &Path) -> Result<Vec<u8>, u32> {
    let units: Vec<u16> = path.as_os_str().encode_wide().collect();
    if !path.is_absolute()
        || units.is_empty()
        || units.contains(&0)
        || units.len() > MAX_PATH_BYTES / 2
    {
        return Err(87);
    }
    Ok(units.into_iter().flat_map(u16::to_le_bytes).collect())
}

pub(super) fn decode_path(bytes: &[u8]) -> Result<PathBuf, u32> {
    if bytes.is_empty() || bytes.len() > MAX_PATH_BYTES || !bytes.len().is_multiple_of(2) {
        return Err(87);
    }
    let units: Vec<_> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    if units.contains(&0) {
        return Err(87);
    }
    let path = PathBuf::from(OsString::from_wide(&units));
    if !path.is_absolute() {
        return Err(87);
    }
    Ok(path)
}
