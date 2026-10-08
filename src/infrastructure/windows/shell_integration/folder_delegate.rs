use super::registry::{
    delete_key_if_empty, delete_value, key_exists, read_value, registry_string_text, write_string,
    write_value, RegistryValueSnapshot, REGISTRY_CLASSES_ROOT,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub(super) const CLSID_TEXT: &str = "{7b9f6e73-c8a1-4c2d-9f18-26b47e5f9c31}";
const THREADING_MODEL: &str = "Apartment";
const DLL_NAME: &str = "mtt_explorer_command.dll";
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
    pub(super) inproc_key_existed: bool,
    pub(super) inproc_server: Option<RegistryValueSnapshot>,
    pub(super) threading_model: Option<RegistryValueSnapshot>,
    pub(super) installed_folder_command: String,
    pub(super) installed_clsid: String,
    pub(super) installed_server_path: String,
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

fn inproc_server_key_path() -> String {
    format!("{}\\InprocServer32", clsid_key_path())
}

fn registry_value_matches_string(value: &Option<RegistryValueSnapshot>, text: &str) -> bool {
    value.as_ref().and_then(registry_string_text).as_deref() == Some(text)
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
    Ok(command_references_mtt || registry_value_matches_string(&delegate_execute, CLSID_TEXT))
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
    let server_path = actual_directory.join(DLL_NAME);
    if !server_path.is_file() {
        return Err(
            "The MTT Folder COM server is missing from the protected installation directory."
                .to_string(),
        );
    }
    Ok(server_path)
}

pub(super) fn capture(
    executable: &Path,
    installed_folder_command: String,
) -> Result<FolderDelegateSnapshot, String> {
    let server_path = require_protected_install(executable)?;
    let clsid_path = clsid_key_path();
    let inproc_path = inproc_server_key_path();
    if key_exists(&clsid_path)? || key_exists(&inproc_path)? {
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
    Ok(FolderDelegateSnapshot {
        classes_root_key_existed: key_exists(&classes_root)?,
        folder_key_existed: key_exists(&folder_key)?,
        shell_key_existed: key_exists(&folder_shell)?,
        open_key_existed: key_exists(&folder_open)?,
        command_key_existed: key_exists(&folder_command_key)?,
        default_command: read_value(&folder_command_key, None)?,
        delegate_execute: read_value(&folder_command_key, Some("DelegateExecute"))?,
        clsid_root_key_existed: key_exists(&clsid_root)?,
        clsid_key_existed: false,
        inproc_key_existed: false,
        inproc_server: None,
        threading_model: None,
        installed_folder_command,
        installed_clsid: CLSID_TEXT.to_string(),
        installed_server_path: server_path.to_string_lossy().into_owned(),
    })
}

fn current_folder_values(
) -> Result<(Option<RegistryValueSnapshot>, Option<RegistryValueSnapshot>), String> {
    let folder_command = folder_command_key_path();
    Ok((
        read_value(&folder_command, None)?,
        read_value(&folder_command, Some("DelegateExecute"))?,
    ))
}

fn current_com_values(
) -> Result<(Option<RegistryValueSnapshot>, Option<RegistryValueSnapshot>), String> {
    let inproc_path = inproc_server_key_path();
    Ok((
        read_value(&inproc_path, None)?,
        read_value(&inproc_path, Some("ThreadingModel"))?,
    ))
}

fn validate_folder_values(snapshot: &FolderDelegateSnapshot) -> Result<(), String> {
    let (default_command, delegate_execute) = current_folder_values()?;
    let command_is_original = default_command == snapshot.default_command;
    let command_is_owned =
        registry_value_matches_string(&default_command, &snapshot.installed_folder_command);
    let delegate_is_original = delegate_execute == snapshot.delegate_execute;
    let delegate_is_owned =
        registry_value_matches_string(&delegate_execute, &snapshot.installed_clsid);
    if (!command_is_original && !command_is_owned) || (!delegate_is_original && !delegate_is_owned)
    {
        return Err(
            "The Folder open command changed outside MTT; it was left untouched.".to_string(),
        );
    }
    Ok(())
}

fn validate_com_values(snapshot: &FolderDelegateSnapshot) -> Result<(), String> {
    let (server, threading_model) = current_com_values()?;
    let server_is_original = server == snapshot.inproc_server;
    let server_is_owned = registry_value_matches_string(&server, &snapshot.installed_server_path);
    let threading_is_original = threading_model == snapshot.threading_model;
    let threading_is_owned = registry_value_matches_string(&threading_model, THREADING_MODEL);
    if (!server_is_original && !server_is_owned) || (!threading_is_original && !threading_is_owned)
    {
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
    let inproc_path = inproc_server_key_path();
    write_string(&inproc_path, None, &snapshot.installed_server_path)?;
    write_string(&inproc_path, Some("ThreadingModel"), THREADING_MODEL)?;

    let folder_command = folder_command_key_path();
    write_string(&folder_command, None, &snapshot.installed_folder_command)?;
    write_string(
        &folder_command,
        Some("DelegateExecute"),
        &snapshot.installed_clsid,
    )
}

fn restore_value_if_owned(
    key_path: &str,
    value_name: Option<&str>,
    current: &Option<RegistryValueSnapshot>,
    original: &Option<RegistryValueSnapshot>,
    installed_string: &str,
) -> Result<bool, String> {
    if registry_value_matches_string(current, installed_string) {
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
        changed |= restore_value_if_owned(
            &folder_command,
            None,
            &current_folder_default,
            &snapshot.default_command,
            &snapshot.installed_folder_command,
        )?;
        changed |= restore_value_if_owned(
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

        let inproc_path = inproc_server_key_path();
        let (current_server, current_threading) = current_com_values()?;
        changed |= restore_value_if_owned(
            &inproc_path,
            None,
            &current_server,
            &snapshot.inproc_server,
            &snapshot.installed_server_path,
        )?;
        changed |= restore_value_if_owned(
            &inproc_path,
            Some("ThreadingModel"),
            &current_threading,
            &snapshot.threading_model,
            THREADING_MODEL,
        )?;

        if !snapshot.inproc_key_existed {
            delete_key_if_empty(&inproc_path)?;
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
            inproc_key_existed: false,
            inproc_server: None,
            threading_model: None,
            installed_folder_command:
                r#""C:\\Program Files\\Example App\\mtt-file-manager.exe" "%1""#.to_string(),
            installed_clsid: CLSID_TEXT.to_string(),
            installed_server_path: r"C:\\Program Files\\Example App\\mtt_explorer_command.dll"
                .to_string(),
        };
        let encoded = serde_json::to_string(&snapshot).unwrap();
        let decoded: FolderDelegateSnapshot = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, snapshot);
    }
}
