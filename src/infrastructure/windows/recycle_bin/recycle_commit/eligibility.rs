use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{
    CloseHandle, LocalFree, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, HANDLE, HLOCAL,
};
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::{
    GetTokenInformation, IsValidSid, TokenUser, TOKEN_QUERY, TOKEN_USER,
};
use windows::Win32::Storage::FileSystem::{
    GetDriveTypeW, GetVolumeInformationW, GetVolumeNameForVolumeMountPointW, FILE_ATTRIBUTE_HIDDEN,
    FILE_ATTRIBUTE_READONLY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_SYSTEM,
};
use windows::Win32::System::Registry::{
    RegGetValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, REG_DWORD, REG_VALUE_TYPE,
    RRF_RT_REG_DWORD,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::Shell::{
    SHGetSetSettings, SHQueryRecycleBinW, SHELLSTATEA, SHQUERYRBINFO, SSF_NOCONFIRMRECYCLE,
};

const EXPLORER_BITBUCKET: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\BitBucket";
const VOLUME_SETTINGS: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\BitBucket\Volume";
const EXPLORER_POLICIES: &str = r"Software\Microsoft\Windows\CurrentVersion\Policies\Explorer";

pub(super) struct RecycleContext {
    pub(super) recycle_directory: PathBuf,
    pub(super) item_size: u64,
}

pub(super) fn prepare(path: &Path, confirmation_already_shown: bool) -> Option<RecycleContext> {
    if !confirmation_already_shown && shell_confirmation_is_enabled().unwrap_or(true) {
        return None;
    }
    if !path.is_absolute()
        || path.file_name().is_none()
        || is_recycle_bin_path(path)
        || crate::infrastructure::security::validate_no_reparse_points(path).is_err()
    {
        return None;
    }
    if registry_dword(HKEY_LOCAL_MACHINE, EXPLORER_BITBUCKET, "NukeOnDelete")
        .ok()?
        .is_some_and(|value| value != 0)
    {
        return None;
    }

    let drive_root = drive_root(path)?;
    let root_wide: Vec<u16> = drive_root
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    if unsafe { GetDriveTypeW(PCWSTR(root_wide.as_ptr())) } != 3 {
        return None;
    }

    let mut file_system = [0u16; 32];
    if unsafe {
        GetVolumeInformationW(
            PCWSTR(root_wide.as_ptr()),
            None,
            None,
            None,
            None,
            Some(&mut file_system),
        )
    }
    .is_err()
        || !String::from_utf16_lossy(&file_system)
            .trim_end_matches('\0')
            .eq_ignore_ascii_case("NTFS")
    {
        return None;
    }

    let volume_guid = volume_guid(&drive_root)?;
    let max_bytes = configured_max_bytes(&volume_guid)?;
    let recycle_root = drive_root.join("$Recycle.Bin");
    let recycle_root_metadata = std::fs::symlink_metadata(&recycle_root).ok()?;
    if !recycle_root_metadata.is_dir() || is_reparse_point(&recycle_root_metadata) {
        return None;
    }

    let recycle_directory = recycle_root.join(current_user_sid()?);
    let user_bin_metadata = std::fs::symlink_metadata(&recycle_directory).ok()?;
    if !user_bin_metadata.is_dir() || is_reparse_point(&user_bin_metadata) {
        return None;
    }

    let source_metadata = std::fs::symlink_metadata(path).ok()?;
    if requires_shell_attributes(&source_metadata)
        || (!source_metadata.is_file() && !source_metadata.is_dir())
    {
        return None;
    }
    let item_size = if source_metadata.is_dir() {
        crate::infrastructure::global_search::folder_size(path)
            .ok()
            .map(|result| result.0)?
    } else {
        source_metadata.len()
    };

    let mut bin_info = SHQUERYRBINFO {
        cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32,
        ..Default::default()
    };
    if unsafe { SHQueryRecycleBinW(PCWSTR(root_wide.as_ptr()), &mut bin_info) }.is_err()
        || bin_info.i64Size < 0
        || (bin_info.i64Size as u64).checked_add(item_size)? > max_bytes
    {
        return None;
    }

    Some(RecycleContext {
        recycle_directory,
        item_size,
    })
}

fn drive_root(path: &Path) -> Option<PathBuf> {
    let value = path.as_os_str().to_string_lossy();
    let value = value.strip_prefix(r"\\?\").unwrap_or(&value);
    let bytes = value.as_bytes();
    if bytes.len() < 3
        || !bytes[0].is_ascii_alphabetic()
        || bytes[1] != b':'
        || !matches!(bytes[2], b'\\' | b'/')
    {
        return None;
    }
    Some(PathBuf::from(format!("{}:\\", bytes[0] as char)))
}

fn is_recycle_bin_path(path: &Path) -> bool {
    path.components().any(|component| {
        let name = component.as_os_str().to_string_lossy();
        name.eq_ignore_ascii_case("$Recycle.Bin")
            || name.eq_ignore_ascii_case("Recycler")
            || name.eq_ignore_ascii_case("Recycled")
    })
}

fn is_reparse_point(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
}

fn requires_shell_attributes(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    // These items may require an additional Shell warning. A normal recycle
    // confirmation does not authorize suppressing warnings for protected items.
    metadata.file_attributes()
        & (FILE_ATTRIBUTE_REPARSE_POINT
            | FILE_ATTRIBUTE_READONLY
            | FILE_ATTRIBUTE_SYSTEM
            | FILE_ATTRIBUTE_HIDDEN)
            .0
        != 0
}

fn volume_guid(drive_root: &Path) -> Option<String> {
    let root: Vec<u16> = drive_root
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let mut name = [0u16; 50];
    if unsafe { GetVolumeNameForVolumeMountPointW(PCWSTR(root.as_ptr()), &mut name) }.is_err() {
        return None;
    }
    let end = name
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(name.len());
    let value = String::from_utf16(&name[..end]).ok()?;
    let start = value.find('{')?;
    let finish = value[start..].find('}')? + start + 1;
    Some(value[start..finish].to_string())
}

fn configured_max_bytes(volume_guid: &str) -> Option<u64> {
    if registry_dword(HKEY_CURRENT_USER, EXPLORER_POLICIES, "NoRecycleFiles")
        .ok()?
        .is_some_and(|value| value != 0)
        || registry_dword(HKEY_LOCAL_MACHINE, EXPLORER_POLICIES, "NoRecycleFiles")
            .ok()?
            .is_some_and(|value| value != 0)
    {
        return None;
    }

    // An enforced size policy is left to Shell, including its precedence rules.
    for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        if registry_dword(root, EXPLORER_POLICIES, "RecycleBinSize")
            .ok()?
            .is_some()
        {
            return None;
        }
    }
    let use_global = use_global_settings(
        registry_dword(HKEY_CURRENT_USER, EXPLORER_BITBUCKET, "UseGlobalSettings").ok()?,
    )?;
    let volume_key = format!(r"{VOLUME_SETTINGS}\{volume_guid}");
    let (settings_key, nuke_setting) = if use_global {
        (
            EXPLORER_BITBUCKET,
            registry_dword(HKEY_CURRENT_USER, EXPLORER_BITBUCKET, "NukeOnDelete").ok()?,
        )
    } else {
        (
            volume_key.as_str(),
            registry_dword(HKEY_CURRENT_USER, &volume_key, "NukeOnDelete").ok()?,
        )
    };
    if nuke_setting != Some(0)
        || registry_dword(HKEY_LOCAL_MACHINE, settings_key, "NukeOnDelete")
            .ok()?
            .is_some_and(|value| value != 0)
    {
        return None;
    }
    let max_megabytes = registry_dword(HKEY_CURRENT_USER, settings_key, "MaxCapacity").ok()??;
    if max_megabytes == 0 {
        return None;
    }
    (max_megabytes as u64).checked_mul(1024 * 1024)
}

fn use_global_settings(value: Option<u32>) -> Option<bool> {
    // Modern per-volume configuration does not require the legacy switch.
    match value {
        None | Some(0) => Some(false),
        Some(1) => Some(true),
        _ => None,
    }
}

fn shell_confirmation_is_enabled() -> Result<bool, ()> {
    if registry_dword(HKEY_CURRENT_USER, EXPLORER_POLICIES, "ConfirmFileDelete")?
        .is_some_and(|value| value != 0)
        || registry_dword(HKEY_LOCAL_MACHINE, EXPLORER_POLICIES, "ConfirmFileDelete")?
            .is_some_and(|value| value != 0)
    {
        return Ok(true);
    }
    let mut shell_state = SHELLSTATEA::default();
    unsafe {
        SHGetSetSettings(Some(&mut shell_state), SSF_NOCONFIRMRECYCLE, false);
    }
    Ok(shell_state._bitfield1 & 0x04 == 0)
}

fn registry_dword(root: HKEY, key: &str, name: &str) -> Result<Option<u32>, ()> {
    let key: Vec<u16> = key.encode_utf16().chain(Some(0)).collect();
    let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    let mut value = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    let mut value_type = REG_VALUE_TYPE(0);
    unsafe {
        let status = RegGetValueW(
            root,
            PCWSTR(key.as_ptr()),
            PCWSTR(name.as_ptr()),
            RRF_RT_REG_DWORD,
            Some(&mut value_type),
            Some((&mut value as *mut u32).cast()),
            Some(&mut size),
        );
        if status == ERROR_FILE_NOT_FOUND || status == ERROR_PATH_NOT_FOUND {
            return Ok(None);
        }
        if status.is_err() {
            return Err(());
        }
    }
    if value_type != REG_DWORD || size != std::mem::size_of::<u32>() as u32 {
        return Err(());
    }
    Ok(Some(value))
}

fn current_user_sid() -> Option<String> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).ok()?;
        let _token = TokenHandle(token);

        let mut required = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut required);
        if required == 0 || required > 4096 {
            return None;
        }
        let mut buffer = vec![0usize; (required as usize).div_ceil(std::mem::size_of::<usize>())];
        GetTokenInformation(
            token,
            TokenUser,
            Some(buffer.as_mut_ptr().cast()),
            required,
            &mut required,
        )
        .ok()?;
        let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
        if !IsValidSid(user.User.Sid).as_bool() {
            return None;
        }
        let mut sid_string = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut sid_string).ok()?;
        let result = sid_string.to_string().ok();
        let _ = LocalFree(Some(HLOCAL(sid_string.0.cast())));
        result
    }
}

struct TokenHandle(HANDLE);

impl Drop for TokenHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::use_global_settings;

    #[test]
    fn absent_legacy_switch_uses_per_volume_settings() {
        assert_eq!(use_global_settings(None), Some(false));
        assert_eq!(use_global_settings(Some(0)), Some(false));
        assert_eq!(use_global_settings(Some(1)), Some(true));
        assert_eq!(use_global_settings(Some(2)), None);
    }
}
