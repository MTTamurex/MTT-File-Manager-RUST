use super::registry::{
    delete_key_if_empty, delete_value, key_exists, read_value, registry_string_text, write_string,
    write_value, RegistryValueSnapshot, REGISTRY_CLASSES_ROOT,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub(super) const CLSID_TEXT: &str = "{2e6a41d7-54b3-4f0e-b8c2-9d17a3e5c6b8}";
const LEGACY_CLSID_TEXT: &str = "{7b9f6e73-c8a1-4c2d-9f18-26b47e5f9c31}";
const SERVER_NAME: &str = "mtt-shell-command.exe";
const APP_NAME: &str = "mtt-file-manager.exe";
const INSTALL_DIRECTORY_NAME: &str = "MTT File Manager";

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub(super) struct FolderDelegateSnapshot {
    pub(super) classes_root_key_existed: bool,
    pub(super) folder_key_existed: bool,
    pub(super) shell_key_existed: bool,
    pub(super) open_key_existed: bool,
    pub(super) command_key_existed: bool,
    pub(super) default_command: Option<RegistryValueSnapshot>,
    pub(super) delegate_execute: Option<RegistryValueSnapshot>,
    pub(super) clsid_root_key_existed: bool,
    pub(super) clsid_key_existed: bool,
    #[serde(default)]
    pub(super) local_server_key_existed: bool,
    #[serde(default)]
    pub(super) local_server: Option<RegistryValueSnapshot>,
    pub(super) installed_folder_command: String,
    pub(super) installed_clsid: String,
    #[serde(default)]
    pub(super) installed_server_command: String,
    #[serde(
        default,
        rename = "installed_server_path",
        skip_serializing_if = "Option::is_none"
    )]
    pub(super) legacy_installed_server_path: Option<String>,
}

fn folder_shell_key_path() -> String {
    format!("{REGISTRY_CLASSES_ROOT}\\Folder\\shell")
}

fn folder_open_key_path() -> String {
    format!("{}\\open", folder_shell_key_path())
}

fn folder_command_key_path() -> String {
    format!("{}\\command", folder_open_key_path())
}

fn clsid_key_path() -> String {
    format!("{REGISTRY_CLASSES_ROOT}\\CLSID\\{CLSID_TEXT}")
}

fn local_server_key_path() -> String {
    format!("{}\\LocalServer32", clsid_key_path())
}

fn legacy_clsid_key_path() -> String {
    format!("{REGISTRY_CLASSES_ROOT}\\CLSID\\{LEGACY_CLSID_TEXT}")
}

fn registry_value_matches_string(value: &Option<RegistryValueSnapshot>, text: &str) -> bool {
    value.as_ref().and_then(registry_string_text).as_deref() == Some(text)
}

fn current_folder_values(
) -> Result<(Option<RegistryValueSnapshot>, Option<RegistryValueSnapshot>), String> {
    let folder_command = folder_command_key_path();
    Ok((
        read_value(&folder_command, None)?,
        read_value(&folder_command, Some("DelegateExecute"))?,
    ))
}

pub(super) fn current_handler_points_to_mtt(command: &str) -> Result<bool, String> {
    let (folder_command, delegate_execute) = current_folder_values()?;
    let executable = command
        .trim_start_matches('"')
        .split('"')
        .next()
        .unwrap_or(command)
        .to_ascii_lowercase();
    let command_references_mtt = folder_command
        .as_ref()
        .and_then(registry_string_text)
        .map(|current| current.to_ascii_lowercase().contains(&executable))
        .unwrap_or(false);
    Ok(command_references_mtt
        || registry_value_matches_string(&delegate_execute, CLSID_TEXT)
        || registry_value_matches_string(&delegate_execute, LEGACY_CLSID_TEXT))
}

fn require_protected_install(executable: &Path) -> Result<PathBuf, String> {
    let executable_name_matches = executable
        .file_name()
        .map(|name| name.to_string_lossy().eq_ignore_ascii_case(APP_NAME))
        .unwrap_or(false);
    if !executable_name_matches {
        return Err("The Folder handler requires the installed MTT executable.".to_string());
    }
    let program_files = std::env::var_os("ProgramFiles")
        .map(PathBuf::from)
        .ok_or_else(|| "The protected Program Files directory could not be located.".to_string())?;
    let expected_directory = program_files.join(INSTALL_DIRECTORY_NAME);
    let actual_directory = executable
        .parent()
        .ok_or_else(|| "The MTT executable directory could not be located.".to_string())?;
    let expected_directory = expected_directory.canonicalize().map_err(|error| {
        format!("The protected MTT installation directory is unavailable: {error}")
    })?;
    let actual_directory = actual_directory
        .canonicalize()
        .map_err(|error| format!("The MTT executable directory could not be resolved: {error}"))?;
    if !actual_directory
        .to_string_lossy()
        .eq_ignore_ascii_case(&expected_directory.to_string_lossy())
    {
        return Err("The Folder handler requires MTT to be installed in its protected Program Files directory.".to_string());
    }
    let server_path = actual_directory.join(SERVER_NAME);
    if !server_path.is_file() {
        return Err(
            "The MTT Folder COM server is missing from the protected installation directory."
                .to_string(),
        );
    }
    Ok(server_path)
}

fn remove_legacy_registration(snapshot: &FolderDelegateSnapshot) -> Result<(), String> {
    let Some(installed_server_path) = &snapshot.legacy_installed_server_path else {
        return Ok(());
    };
    let legacy_clsid = legacy_clsid_key_path();
    let legacy_inproc = format!("{legacy_clsid}\\InprocServer32");
    let current_server = read_value(&legacy_inproc, None)?;
    if registry_value_matches_string(&current_server, installed_server_path) {
        delete_value(&legacy_inproc, None)?;
        if registry_value_matches_string(
            &read_value(&legacy_inproc, Some("ThreadingModel"))?,
            "Apartment",
        ) {
            delete_value(&legacy_inproc, Some("ThreadingModel"))?;
        }
    }
    delete_key_if_empty(&legacy_inproc)?;
    delete_key_if_empty(&legacy_clsid)?;
    Ok(())
}

fn legacy_references_mtt(
    default_command: &Option<RegistryValueSnapshot>,
    delegate_execute: &Option<RegistryValueSnapshot>,
) -> bool {
    registry_value_matches_string(delegate_execute, LEGACY_CLSID_TEXT)
        || default_command
            .as_ref()
            .and_then(registry_string_text)
            .map(|command| {
                command
                    .to_ascii_lowercase()
                    .contains(&APP_NAME.to_ascii_lowercase())
            })
            .unwrap_or(false)
}

pub(super) fn capture(
    executable: &Path,
    installed_folder_command: String,
) -> Result<FolderDelegateSnapshot, String> {
    let server_path = require_protected_install(executable)?;
    let clsid_path = clsid_key_path();
    let local_server_path = local_server_key_path();
    if key_exists(&clsid_path)? || key_exists(&local_server_path)? {
        return Err(
            "The MTT Folder COM class identifier is already registered; it was left untouched."
                .to_string(),
        );
    }
    let classes_root = REGISTRY_CLASSES_ROOT.to_string();
    let folder_key = format!("{REGISTRY_CLASSES_ROOT}\\Folder");
    let folder_shell = folder_shell_key_path();
    let folder_open = folder_open_key_path();
    let folder_command_key = folder_command_key_path();
    let clsid_root = format!("{REGISTRY_CLASSES_ROOT}\\CLSID");
    let current_default = read_value(&folder_command_key, None)?;
    let current_delegate = read_value(&folder_command_key, Some("DelegateExecute"))?;
    if legacy_references_mtt(&current_default, &current_delegate) {
        return Err("The previous MTT Folder delegate is still active but its recovery snapshot is missing.".to_string());
    }
    Ok(FolderDelegateSnapshot {
        classes_root_key_existed: key_exists(&classes_root)?,
        folder_key_existed: key_exists(&folder_key)?,
        shell_key_existed: key_exists(&folder_shell)?,
        open_key_existed: key_exists(&folder_open)?,
        command_key_existed: key_exists(&folder_command_key)?,
        default_command: current_default,
        delegate_execute: current_delegate,
        clsid_root_key_existed: key_exists(&clsid_root)?,
        clsid_key_existed: false,
        local_server_key_existed: false,
        local_server: None,
        installed_folder_command,
        installed_clsid: CLSID_TEXT.to_string(),
        installed_server_command: format!("\"{}\"", server_path.to_string_lossy()),
        legacy_installed_server_path: None,
    })
}

fn current_com_values() -> Result<Option<RegistryValueSnapshot>, String> {
    read_value(&local_server_key_path(), None)
}

fn value_still_installed(current: &Option<RegistryValueSnapshot>, installed_string: &str) -> bool {
    registry_value_matches_string(current, installed_string)
}

fn validate_folder_values(snapshot: &FolderDelegateSnapshot) -> Result<(), String> {
    let (default_command, delegate_execute) = current_folder_values()?;
    let command_is_installed =
        value_still_installed(&default_command, &snapshot.installed_folder_command);
    let delegate_is_installed = value_still_installed(&delegate_execute, &snapshot.installed_clsid);
    let command_is_untouched = default_command == snapshot.default_command;
    let delegate_is_untouched = delegate_execute == snapshot.delegate_execute;
    if (!command_is_installed && !command_is_untouched)
        || (!delegate_is_installed && !delegate_is_untouched)
    {
        return Err(
            "The Folder open command changed outside MTT; it was left untouched.".to_string(),
        );
    }
    Ok(())
}

fn validate_com_values(snapshot: &FolderDelegateSnapshot) -> Result<(), String> {
    let server = current_com_values()?;
    let server_is_installed = value_still_installed(&server, &snapshot.installed_server_command);
    let server_is_untouched = server == snapshot.local_server;
    if !server_is_installed && !server_is_untouched {
        return Err(
            "The MTT Folder COM registration changed outside MTT; it was left untouched."
                .to_string(),
        );
    }
    Ok(())
}

pub(super) fn validate(snapshot: &FolderDelegateSnapshot) -> Result<(), String> {
    validate_folder_values(snapshot)?;
    validate_com_values(snapshot)
}

pub(super) fn enable(snapshot: &FolderDelegateSnapshot) -> Result<(), String> {
    validate(snapshot)?;
    write_string(
        &local_server_key_path(),
        None,
        &snapshot.installed_server_command,
    )?;

    let folder_command = folder_command_key_path();
    write_string(&folder_command, None, &snapshot.installed_folder_command)?;
    write_string(
        &folder_command,
        Some("DelegateExecute"),
        &snapshot.installed_clsid,
    )
}

fn restore_value_if_installed(
    key_path: &str,
    value_name: Option<&str>,
    current: &Option<RegistryValueSnapshot>,
    original: &Option<RegistryValueSnapshot>,
    installed_string: &str,
) -> Result<bool, String> {
    if value_still_installed(current, installed_string) {
        match original {
            Some(value) => write_value(key_path, value_name, value)?,
            None => delete_value(key_path, value_name)?,
        }
        Ok(true)
    } else {
        Ok(false)
    }
}

pub(super) fn restore(snapshot: &FolderDelegateSnapshot) -> Result<bool, String> {
    let folder_command = folder_command_key_path();
    let mut changed = false;
    let restore_result: Result<(), String> = (|| {
        let (current_folder_default, current_folder_delegate) = current_folder_values()?;
        changed |= restore_value_if_installed(
            &folder_command,
            None,
            &current_folder_default,
            &snapshot.default_command,
            &snapshot.installed_folder_command,
        )?;
        changed |= restore_value_if_installed(
            &folder_command,
            Some("DelegateExecute"),
            &current_folder_delegate,
            &snapshot.delegate_execute,
            &snapshot.installed_clsid,
        )?;

        if current_handler_points_to_mtt(&snapshot.installed_folder_command)? {
            return Err(
                "The Folder handler still references MTT; its COM registration was retained."
                    .to_string(),
            );
        }
        remove_legacy_registration(snapshot)?;

        let local_server_path = local_server_key_path();
        let current_server = current_com_values()?;
        changed |= restore_value_if_installed(
            &local_server_path,
            None,
            &current_server,
            &snapshot.local_server,
            &snapshot.installed_server_command,
        )?;

        if !snapshot.local_server_key_existed {
            delete_key_if_empty(&local_server_path)?;
        }
        if !snapshot.clsid_key_existed {
            delete_key_if_empty(&clsid_key_path())?;
        }
        if !snapshot.clsid_root_key_existed {
            delete_key_if_empty(&format!("{REGISTRY_CLASSES_ROOT}\\CLSID"))?;
        }
        if !snapshot.command_key_existed {
            delete_key_if_empty(&folder_command)?;
        }
        if !snapshot.open_key_existed {
            delete_key_if_empty(&folder_open_key_path())?;
        }
        if !snapshot.shell_key_existed {
            delete_key_if_empty(&folder_shell_key_path())?;
        }
        if !snapshot.folder_key_existed {
            delete_key_if_empty(&format!("{REGISTRY_CLASSES_ROOT}\\Folder"))?;
        }
        if !snapshot.classes_root_key_existed {
            delete_key_if_empty(REGISTRY_CLASSES_ROOT)?;
        }
        Ok(())
    })();
    if changed {
        super::registry::notify_association_changed();
    }
    restore_result?;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::super::registry::registry_string;
    use super::{FolderDelegateSnapshot, CLSID_TEXT};

    #[test]
    fn folder_delegate_snapshot_preserves_original_registry_values() {
        let snapshot = FolderDelegateSnapshot {
            classes_root_key_existed: true,
            folder_key_existed: true,
            shell_key_existed: true,
            open_key_existed: true,
            command_key_existed: true,
            default_command: Some(registry_string(r"%SystemRoot%\\Explorer.exe")),
            delegate_execute: Some(registry_string("{stock-folder-delegate}")),
            clsid_root_key_existed: true,
            clsid_key_existed: false,
            local_server_key_existed: false,
            local_server: None,
            installed_folder_command:
                r#""C:\\Program Files\\Example App\\mtt-file-manager.exe" "%1""#.to_string(),
            installed_clsid: CLSID_TEXT.to_string(),
            installed_server_command: r#""C:\\Program Files\\Example App\\mtt-shell-command.exe""#
                .to_string(),
            legacy_installed_server_path: None,
        };
        let encoded = serde_json::to_string(&snapshot).unwrap();
        let decoded: FolderDelegateSnapshot = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, snapshot);
    }
}
