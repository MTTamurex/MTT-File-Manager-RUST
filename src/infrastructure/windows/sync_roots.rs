use crate::domain::cloud_root::CloudRoot;
use std::collections::HashSet;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use windows::core::{Interface, PCWSTR, PWSTR};
use windows::Win32::Foundation::{ERROR_NO_MORE_ITEMS, ERROR_SUCCESS};
use windows::Win32::Storage::FileSystem::{
    GetFileAttributesW, GetVolumeInformationW, QueryDosDeviceW, FILE_ATTRIBUTE_DIRECTORY,
    INVALID_FILE_ATTRIBUTES,
};
use windows::Win32::System::Com::{
    CoCreateInstance, IPersistFile, CLSCTX_INPROC_SERVER, STGM_READ,
};
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegEnumValueW, RegGetValueW, RegOpenKeyExW, HKEY,
    HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, REG_VALUE_TYPE, RRF_RT_REG_EXPAND_SZ,
    RRF_RT_REG_SZ,
};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink, SLGP_UNCPRIORITY};

const SYNC_ROOT_MANAGER_KEY: &str =
    "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Explorer\\SyncRootManager";
const FILE_SUPPORTS_REMOTE_STORAGE: u32 = 0x0000_0100;

struct RegKey(HKEY);

impl Drop for RegKey {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

/// Returns Cloud Files sync roots registered with Windows Explorer.
///
/// Providers such as Proton Drive and OneDrive are exposed as Cloud Files sync
/// roots / Shell namespace items, not necessarily as logical volumes, so
/// `GetLogicalDriveStringsW` does not cover this Explorer sidebar category.
pub fn get_cloud_sync_roots() -> Vec<CloudRoot> {
    let mut roots = Vec::new();
    let mut seen_paths = HashSet::new();

    for root in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        let Some(manager_key) = open_key(root, SYNC_ROOT_MANAGER_KEY) else {
            continue;
        };

        for provider_key_name in enum_subkeys(manager_key.0) {
            let provider_path = format!("{}\\{}", SYNC_ROOT_MANAGER_KEY, provider_key_name);
            let Some(provider_key) = open_key(root, &provider_path) else {
                continue;
            };

            let display_name = query_string(provider_key.0, "DisplayNameResource");
            let icon_resource = query_string(provider_key.0, "IconResource");

            let user_roots_path = format!("{}\\UserSyncRoots", provider_path);
            let Some(user_roots_key) = open_key(root, &user_roots_path) else {
                continue;
            };

            for path in enum_string_values(user_roots_key.0) {
                let normalized_path = path.trim_end_matches(['\\', '/']).to_string();
                if normalized_path.is_empty() {
                    continue;
                }

                let dedupe_key = normalized_path.to_lowercase();
                if !seen_paths.insert(dedupe_key) {
                    continue;
                }

                roots.push(CloudRoot::windows_cloud_files(
                    normalized_path,
                    display_name
                        .clone()
                        .filter(|label| !label.trim().is_empty())
                        .unwrap_or_else(|| provider_display_name(&provider_key_name)),
                    icon_resource.clone().filter(|s| !s.trim().is_empty()),
                ));
            }
        }
    }

    roots.sort_by(|a, b| a.label.cmp(&b.label).then_with(|| a.path.cmp(&b.path)));
    roots
}

pub fn get_drives_and_cloud_roots() -> (Vec<(String, String)>, Vec<CloudRoot>) {
    let disks = super::drives::get_all_drives();
    let mut cloud_roots = get_cloud_sync_roots();

    cloud_roots.extend(get_google_drive_shortcut_roots(&disks));

    let hidden_drive_roots: HashSet<String> = cloud_roots
        .iter()
        .filter_map(|root| root.source_path.as_deref())
        .map(normalize_drive_root_for_compare)
        .collect();

    let visible_disks = disks
        .into_iter()
        .filter(|(path, _)| !hidden_drive_roots.contains(&normalize_drive_root_for_compare(path)))
        .collect();

    cloud_roots.sort_by(|a, b| a.label.cmp(&b.label).then_with(|| a.path.cmp(&b.path)));
    (visible_disks, cloud_roots)
}

fn get_google_drive_shortcut_roots(disks: &[(String, String)]) -> Vec<CloudRoot> {
    let mut roots = Vec::new();
    let mut seen_targets = HashSet::new();

    for (drive_path, drive_label) in disks {
        if !is_google_drivefs_volume(drive_path, drive_label) {
            continue;
        }

        let Ok(entries) = std::fs::read_dir(drive_path) else {
            continue;
        };

        for entry in entries.flatten() {
            let shortcut_path = entry.path();
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => continue,
            };

            if file_type.is_dir() {
                if is_hidden_or_system(&shortcut_path) {
                    continue;
                }

                push_google_drive_root(
                    &mut roots,
                    &mut seen_targets,
                    shortcut_path.clone(),
                    entry.file_name().to_string_lossy().as_ref(),
                    Some(shortcut_path.to_string_lossy().to_string()),
                    drive_path.clone(),
                );
                continue;
            }

            if !shortcut_path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("lnk"))
            {
                continue;
            }

            let Some(target) = resolve_shortcut_target(&shortcut_path) else {
                continue;
            };
            if !fast_is_directory(&target) {
                continue;
            }

            let shortcut_name = shortcut_path
                .file_stem()
                .and_then(|name| name.to_str())
                .filter(|name| !name.trim().is_empty())
                .unwrap_or("My Drive");

            push_google_drive_root(
                &mut roots,
                &mut seen_targets,
                target,
                shortcut_name,
                Some(shortcut_path.to_string_lossy().to_string()),
                drive_path.clone(),
            );
        }
    }

    roots
}

fn push_google_drive_root(
    roots: &mut Vec<CloudRoot>,
    seen_targets: &mut HashSet<String>,
    target: PathBuf,
    name: &str,
    icon_resource: Option<String>,
    source_path: String,
) {
    let target_text = target
        .to_string_lossy()
        .trim_end_matches(['\\', '/'])
        .to_string();
    if target_text.is_empty() || !seen_targets.insert(target_text.to_lowercase()) {
        return;
    }

    let display_name = if name.trim().is_empty() {
        "Google Drive".to_string()
    } else {
        format!("Google Drive - {}", name.trim())
    };

    roots.push(CloudRoot::google_drive_shortcut(
        target_text,
        display_name,
        icon_resource,
        source_path,
    ));
}

fn is_google_drivefs_volume(drive_path: &str, drive_label: &str) -> bool {
    if !drive_label_is_google_drive(drive_label) {
        return false;
    }

    let file_system = super::drives::get_volume_info(drive_path).file_system;
    if text_mentions_drivefs(&file_system) {
        return true;
    }

    query_dos_device(drive_path).is_some_and(|device| text_mentions_drivefs(&device))
        || volume_supports_remote_storage(drive_path)
}

fn drive_label_is_google_drive(drive_label: &str) -> bool {
    let label = drive_label
        .rfind(" (")
        .map(|idx| &drive_label[..idx])
        .unwrap_or(drive_label)
        .trim();

    label.eq_ignore_ascii_case("Google Drive")
}

fn text_mentions_drivefs(text: &str) -> bool {
    let normalized = text.to_lowercase();
    normalized.contains("drivefs") || normalized.contains("google")
}

fn query_dos_device(drive_path: &str) -> Option<String> {
    let drive_name = drive_path.trim_end_matches(['\\', '/']);
    if drive_name.len() != 2 || !drive_name.ends_with(':') {
        return None;
    }

    let drive_wide: Vec<u16> = drive_name
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut buffer = vec![0u16; 1024];
    let len = unsafe { QueryDosDeviceW(PCWSTR(drive_wide.as_ptr()), Some(&mut buffer)) };
    if len == 0 {
        return None;
    }

    Some(String::from_utf16_lossy(&buffer[..len as usize]))
}

fn volume_supports_remote_storage(drive_path: &str) -> bool {
    let path_wide: Vec<u16> = drive_path
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut file_system_flags = 0u32;

    unsafe {
        GetVolumeInformationW(
            PCWSTR(path_wide.as_ptr()),
            None,
            None,
            None,
            Some(&mut file_system_flags),
            None,
        )
        .is_ok()
            && (file_system_flags & FILE_SUPPORTS_REMOTE_STORAGE) != 0
    }
}

fn resolve_shortcut_target(shortcut_path: &Path) -> Option<PathBuf> {
    let _com =
        crate::infrastructure::windows::recycle_bin::ComApartmentGuard::init_sta_best_effort();

    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let persist: IPersistFile = link.cast().ok()?;
        let shortcut_wide: Vec<u16> = shortcut_path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        persist
            .Load(PCWSTR(shortcut_wide.as_ptr()), STGM_READ)
            .ok()?;

        let mut target = vec![0u16; 32_768];
        link.GetPath(&mut target, std::ptr::null_mut(), SLGP_UNCPRIORITY.0 as u32)
            .ok()?;
        let nul = target
            .iter()
            .position(|ch| *ch == 0)
            .unwrap_or(target.len());
        if nul == 0 {
            return None;
        }

        let target_text = String::from_utf16_lossy(&target[..nul]);
        Some(PathBuf::from(expand_environment_strings(&target_text)))
    }
}

fn expand_environment_strings(text: &str) -> String {
    if !text.contains('%') {
        return text.to_string();
    }

    let mut output = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(start) = rest.find('%') {
        output.push_str(&rest[..start]);
        let after_start = &rest[start + 1..];
        let Some(end) = after_start.find('%') else {
            output.push('%');
            output.push_str(after_start);
            return output;
        };

        let name = &after_start[..end];
        if name.is_empty() {
            output.push_str("%%");
        } else if let Ok(value) = std::env::var(name) {
            output.push_str(&value);
        } else {
            output.push('%');
            output.push_str(name);
            output.push('%');
        }

        rest = &after_start[end + 1..];
    }

    output.push_str(rest);
    output
}

fn fast_is_directory(path: &Path) -> bool {
    let path_wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let attrs = unsafe { GetFileAttributesW(PCWSTR(path_wide.as_ptr())) };
    attrs != INVALID_FILE_ATTRIBUTES && (attrs & FILE_ATTRIBUTE_DIRECTORY.0) != 0
}

fn is_hidden_or_system(path: &Path) -> bool {
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x0000_0002;
    const FILE_ATTRIBUTE_SYSTEM: u32 = 0x0000_0004;

    let path_wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let attrs = unsafe { GetFileAttributesW(PCWSTR(path_wide.as_ptr())) };
    attrs == INVALID_FILE_ATTRIBUTES
        || (attrs & (FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM)) != 0
}

fn normalize_drive_root_for_compare(path: &str) -> String {
    path.to_lowercase()
        .trim_end_matches(['\\', '/'])
        .to_string()
}

fn provider_display_name(provider_key_name: &str) -> String {
    provider_key_name
        .split('!')
        .next()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("Cloud Drive")
        .to_string()
}

fn open_key(root: HKEY, path: &str) -> Option<RegKey> {
    let path_wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let mut hkey = HKEY::default();
    unsafe {
        if RegOpenKeyExW(
            root,
            PCWSTR(path_wide.as_ptr()),
            Some(0),
            KEY_READ,
            &mut hkey,
        )
        .is_ok()
        {
            Some(RegKey(hkey))
        } else {
            None
        }
    }
}

fn enum_subkeys(key: HKEY) -> Vec<String> {
    let mut names = Vec::new();
    let mut index = 0u32;

    loop {
        let mut buffer = vec![0u16; 256];
        let mut len = buffer.len() as u32;
        let status = unsafe {
            RegEnumKeyExW(
                key,
                index,
                Some(PWSTR(buffer.as_mut_ptr())),
                &mut len,
                None,
                None,
                None,
                None,
            )
        };

        if status == ERROR_NO_MORE_ITEMS {
            break;
        }
        if status == ERROR_SUCCESS {
            names.push(String::from_utf16_lossy(&buffer[..len as usize]));
        }
        index = index.saturating_add(1);
    }

    names
}

fn enum_string_values(key: HKEY) -> Vec<String> {
    let mut values = Vec::new();
    let mut index = 0u32;

    loop {
        let mut name = vec![0u16; 256];
        let mut name_len = name.len() as u32;
        let mut value_type = 0u32;
        let mut data = vec![0u8; 1024];
        let mut data_len = data.len() as u32;
        let status = unsafe {
            RegEnumValueW(
                key,
                index,
                Some(PWSTR(name.as_mut_ptr())),
                &mut name_len,
                None,
                Some(&mut value_type),
                Some(data.as_mut_ptr()),
                Some(&mut data_len),
            )
        };

        if status == ERROR_NO_MORE_ITEMS {
            break;
        }
        if status == ERROR_SUCCESS && (value_type == 1 || value_type == 2) {
            if let Some(value) = utf16_bytes_to_string(&data[..data_len as usize]) {
                values.push(value);
            }
        }
        index = index.saturating_add(1);
    }

    values
}

fn query_string(key: HKEY, value_name: &str) -> Option<String> {
    let value_wide: Vec<u16> = value_name
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ;

    let mut size = 0u32;
    let mut value_type = REG_VALUE_TYPE(0);
    unsafe {
        if RegGetValueW(
            key,
            PCWSTR::null(),
            PCWSTR(value_wide.as_ptr()),
            flags,
            Some(&mut value_type),
            None,
            Some(&mut size),
        )
        .is_err()
        {
            return None;
        }

        if size == 0 {
            return None;
        }

        let mut data = vec![0u8; size as usize];
        if RegGetValueW(
            key,
            PCWSTR::null(),
            PCWSTR(value_wide.as_ptr()),
            flags,
            Some(&mut value_type),
            Some(data.as_mut_ptr() as *mut _),
            Some(&mut size),
        )
        .is_err()
        {
            return None;
        }

        utf16_bytes_to_string(&data[..size as usize])
    }
}

fn utf16_bytes_to_string(bytes: &[u8]) -> Option<String> {
    if bytes.len() < 2 {
        return None;
    }
    let mut units = Vec::with_capacity(bytes.len() / 2);
    for chunk in bytes.chunks_exact(2) {
        let unit = u16::from_le_bytes([chunk[0], chunk[1]]);
        if unit == 0 {
            break;
        }
        units.push(unit);
    }
    if units.is_empty() {
        None
    } else {
        Some(String::from_utf16_lossy(&units))
    }
}
