use super::folder_delegate::{self, FolderDelegateSnapshot};
use super::registry::{
    command_for_path, current_executable, delete_key_if_empty, delete_value, key_exists,
    notify_association_changed, read_value, registry_string_text, write_string, write_value,
    RegistryValueSnapshot, REGISTRY_CLASSES_ROOT, SHELL_INTEGRATION_STATE_KEY,
};
use crate::infrastructure::app_state_db::AppStateDb;
use serde::{Deserialize, Serialize};
use std::path::Path;

const DEFAULT_HANDLER_SNAPSHOT_KEY: &str = "mtt_default_file_manager_snapshot_v1";
const DEFAULT_HANDLER_CLASSES: [&str; 2] = ["Directory", "Drive"];

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
struct OpenVerbSnapshot {
    class_name: String,
    shell_key_existed: bool,
    open_key_existed: bool,
    command_key_existed: bool,
    default_command: Option<RegistryValueSnapshot>,
    delegate_execute: Option<RegistryValueSnapshot>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
struct DefaultHandlerSnapshot {
    version: u32,
    installed_commands: Vec<String>,
    verbs: Vec<OpenVerbSnapshot>,
    #[serde(default)]
    folder_delegate: Option<FolderDelegateSnapshot>,
}

fn open_key_path(class_name: &str) -> String {
    format!("{REGISTRY_CLASSES_ROOT}\\{class_name}\\shell\\open")
}

fn command_key_path(class_name: &str) -> String {
    format!("{}\\command", open_key_path(class_name))
}

fn capture_open_verb(class_name: &str) -> Result<OpenVerbSnapshot, String> {
    let shell_key_path = format!("{REGISTRY_CLASSES_ROOT}\\{class_name}\\shell");
    let open_key_path = open_key_path(class_name);
    let command_key_path = command_key_path(class_name);
    Ok(OpenVerbSnapshot {
        class_name: class_name.to_string(),
        shell_key_existed: key_exists(&shell_key_path)?,
        open_key_existed: key_exists(&open_key_path)?,
        command_key_existed: key_exists(&command_key_path)?,
        default_command: read_value(&command_key_path, None)?,
        delegate_execute: read_value(&command_key_path, Some("DelegateExecute"))?,
    })
}

fn snapshot_is_owned(
    snapshot: &DefaultHandlerSnapshot,
    verb: &OpenVerbSnapshot,
) -> Result<bool, String> {
    let command_key_path = command_key_path(&verb.class_name);
    let current_command = read_value(&command_key_path, None)?;
    let current_delegate = read_value(&command_key_path, Some("DelegateExecute"))?;
    let command_is_owned = current_command
        .as_ref()
        .and_then(registry_string_text)
        .map(|command| snapshot.installed_commands.contains(&command))
        .unwrap_or(false);
    Ok(command_is_owned && current_delegate.is_none())
}

fn snapshot_is_original(verb: &OpenVerbSnapshot) -> Result<bool, String> {
    let command_key_path = command_key_path(&verb.class_name);
    Ok(read_value(&command_key_path, None)? == verb.default_command
        && read_value(&command_key_path, Some("DelegateExecute"))? == verb.delegate_execute)
}

fn capture_default_handler_snapshot(
    executable: &Path,
    command: String,
) -> Result<DefaultHandlerSnapshot, String> {
    let verbs = DEFAULT_HANDLER_CLASSES
        .iter()
        .map(|&class_name| capture_open_verb(class_name))
        .collect::<Result<Vec<_>, _>>()?;
    let folder_delegate = folder_delegate::capture(executable, command.clone())?;
    Ok(DefaultHandlerSnapshot {
        version: 3,
        installed_commands: vec![command],
        verbs,
        folder_delegate: Some(folder_delegate),
    })
}

fn load_default_handler_snapshot(
    db: &AppStateDb,
) -> Result<Option<DefaultHandlerSnapshot>, String> {
    let raw = db
        .get_preference(DEFAULT_HANDLER_SNAPSHOT_KEY)
        .filter(|value| !value.is_empty());
    let raw = match raw {
        Some(raw) => Some(raw),
        None => read_value(SHELL_INTEGRATION_STATE_KEY, Some("DefaultHandlerSnapshot"))?
            .and_then(|value| registry_string_text(&value)),
    };
    let Some(raw) = raw else {
        return Ok(None);
    };
    let snapshot: DefaultHandlerSnapshot = serde_json::from_str(&raw)
        .map_err(|error| format!("Could not read the saved folder-handler snapshot: {error}"))?;
    if snapshot.version != 1 && snapshot.version != 2 && snapshot.version != 3 {
        return Err("The saved folder-handler snapshot has an unsupported version".to_string());
    }
    Ok(Some(snapshot))
}

fn save_default_handler_snapshot(
    db: &AppStateDb,
    snapshot: &DefaultHandlerSnapshot,
) -> Result<(), String> {
    let raw = serde_json::to_string(snapshot)
        .map_err(|error| format!("Could not encode the folder-handler snapshot: {error}"))?;
    db.set_preference(DEFAULT_HANDLER_SNAPSHOT_KEY, &raw)
        .map_err(|error| format!("Could not save the folder-handler snapshot: {error}"))?;
    write_string(
        SHELL_INTEGRATION_STATE_KEY,
        Some("DefaultHandlerSnapshot"),
        &raw,
    )
}

fn clear_default_handler_snapshot(db: &AppStateDb) {
    if let Err(error) = db.set_preference(DEFAULT_HANDLER_SNAPSHOT_KEY, "") {
        log::warn!(
            "[SHELL-INTEGRATION] Could not clear the restored folder-handler snapshot: {error}"
        );
    }
    if let Err(error) = delete_value(SHELL_INTEGRATION_STATE_KEY, Some("DefaultHandlerSnapshot")) {
        log::warn!("[SHELL-INTEGRATION] Could not clear the registry recovery snapshot: {error}");
    }
    if let Err(error) = delete_key_if_empty(SHELL_INTEGRATION_STATE_KEY) {
        log::warn!("[SHELL-INTEGRATION] Could not clean the state registry key: {error}");
    }
}

fn restore_default_file_manager(
    db: &AppStateDb,
    snapshot: &DefaultHandlerSnapshot,
) -> Result<(), String> {
    let mut states = Vec::with_capacity(snapshot.verbs.len());
    for verb in &snapshot.verbs {
        if snapshot_is_owned(snapshot, verb)? {
            states.push(true);
        } else if snapshot_is_original(verb)? {
            states.push(false);
        } else {
            return Err(
                "A folder-opening command changed outside MTT; it was left untouched.".to_string(),
            );
        }
    }
    if let Some(folder_delegate) = &snapshot.folder_delegate {
        folder_delegate::validate(folder_delegate)?;
    }

    let mut changed = false;
    let restore_result: Result<(), String> = (|| {
        for (verb, is_owned) in snapshot.verbs.iter().zip(states) {
            if !is_owned {
                continue;
            }
            changed = true;
            let command_key_path = command_key_path(&verb.class_name);
            match &verb.default_command {
                Some(value) => write_value(&command_key_path, None, value)?,
                None => delete_value(&command_key_path, None)?,
            }
            match &verb.delegate_execute {
                Some(value) => write_value(&command_key_path, Some("DelegateExecute"), value)?,
                None => delete_value(&command_key_path, Some("DelegateExecute"))?,
            }

            let open_key_path = open_key_path(&verb.class_name);
            let shell_key_path = format!("{REGISTRY_CLASSES_ROOT}\\{}\\shell", verb.class_name);
            if !verb.command_key_existed {
                delete_key_if_empty(&command_key_path)?;
            }
            if !verb.open_key_existed {
                delete_key_if_empty(&open_key_path)?;
            }
            if !verb.shell_key_existed {
                delete_key_if_empty(&shell_key_path)?;
            }
        }
        if let Some(folder_delegate) = &snapshot.folder_delegate {
            changed |= folder_delegate::restore(folder_delegate)?;
        }
        Ok(())
    })();
    if changed {
        notify_association_changed();
    }
    restore_result?;
    clear_default_handler_snapshot(db);
    Ok(())
}

fn set_default_file_manager_enabled(db: &AppStateDb, enabled: bool) -> Result<(), String> {
    let executable = current_executable()?;
    let command = command_for_path(&executable);

    if enabled {
        let mut snapshot = match load_default_handler_snapshot(db)? {
            Some(snapshot) if snapshot.version < 3 || snapshot.folder_delegate.is_none() => {
                restore_default_file_manager(db, &snapshot)?;
                let snapshot = capture_default_handler_snapshot(&executable, command.clone())?;
                save_default_handler_snapshot(db, &snapshot)?;
                snapshot
            }
            Some(snapshot) => snapshot,
            None => {
                let snapshot = capture_default_handler_snapshot(&executable, command.clone())?;
                save_default_handler_snapshot(db, &snapshot)?;
                snapshot
            }
        };

        for verb in &snapshot.verbs {
            if !snapshot_is_owned(&snapshot, verb)? && !snapshot_is_original(verb)? {
                return Err(
                    "A folder-opening command changed outside MTT; no registry values were overwritten."
                        .to_string(),
                );
            }
        }
        let folder_delegate = snapshot
            .folder_delegate
            .as_ref()
            .ok_or_else(|| "The Folder COM delegate snapshot is missing".to_string())?;
        folder_delegate::validate(folder_delegate)?;
        if !snapshot.installed_commands.contains(&command) {
            snapshot.installed_commands.push(command.clone());
            save_default_handler_snapshot(db, &snapshot)?;
        }

        let result = (|| {
            for verb in &snapshot.verbs {
                let command_key_path = command_key_path(&verb.class_name);
                write_string(&command_key_path, None, &command)?;
                delete_value(&command_key_path, Some("DelegateExecute"))?;
            }
            folder_delegate::enable(folder_delegate)
        })();
        if let Err(error) = result {
            if let Err(rollback_error) = restore_default_file_manager(db, &snapshot) {
                return Err(format!("{error}; rollback failed: {rollback_error}"));
            }
            return Err(error);
        }
        notify_association_changed();
        return Ok(());
    }

    match load_default_handler_snapshot(db)? {
        Some(snapshot) => restore_default_file_manager(db, &snapshot),
        None => {
            for class_name in DEFAULT_HANDLER_CLASSES {
                let current_command = read_value(&command_key_path(class_name), None)?
                    .as_ref()
                    .and_then(registry_string_text);
                if current_command.as_deref() == Some(&command) {
                    return Err(
                        "The MTT folder handler is active but its restoration snapshot is missing."
                            .to_string(),
                    );
                }
            }
            if folder_delegate::current_handler_points_to_mtt(&command)? {
                return Err(
                    "The MTT Folder delegate is active but its restoration snapshot is missing."
                        .to_string(),
                );
            }
            Ok(())
        }
    }
}

pub fn set_default_file_manager_enabled_for_current_user(
    db: &AppStateDb,
    enabled: bool,
) -> Result<(), String> {
    set_default_file_manager_enabled(db, enabled)
}

#[cfg(test)]
mod tests {
    use super::super::folder_delegate::{FolderDelegateSnapshot, CLSID_TEXT};
    use super::{DefaultHandlerSnapshot, DEFAULT_HANDLER_CLASSES};

    #[test]
    fn default_handler_covers_directory_and_drive_progs() {
        assert_eq!(DEFAULT_HANDLER_CLASSES, ["Directory", "Drive"]);
    }

    #[test]
    fn version_one_snapshot_loads_without_a_folder_delegate() {
        let legacy: DefaultHandlerSnapshot =
            serde_json::from_str(r#"{"version":1,"installed_commands":[],"verbs":[]}"#).unwrap();
        assert_eq!(legacy.version, 1);
        assert!(legacy.folder_delegate.is_none());
    }

    #[test]
    fn version_two_snapshot_keeps_legacy_inproc_server_path() {
        let legacy: DefaultHandlerSnapshot = serde_json::from_str(
            r#"{"version":2,"installed_commands":[],"verbs":[],"folder_delegate":{"classes_root_key_existed":true,"folder_key_existed":true,"shell_key_existed":true,"open_key_existed":true,"command_key_existed":true,"default_command":null,"delegate_execute":null,"clsid_root_key_existed":true,"clsid_key_existed":false,"inproc_key_existed":false,"inproc_server":null,"threading_model":null,"installed_folder_command":"\"C:\\Program Files\\Example App\\mtt-file-manager.exe\" \"%1\"","installed_clsid":"{7b9f6e73-c8a1-4c2d-9f18-26b47e5f9c31}","installed_server_path":"C:\\Program Files\\Example App\\mtt_explorer_command.dll"}}"#,
        )
        .unwrap();
        let folder_delegate = legacy.folder_delegate.unwrap();
        assert_eq!(legacy.version, 2);
        assert_eq!(
            folder_delegate.legacy_installed_server_path.as_deref(),
            Some(r"C:\Program Files\Example App\mtt_explorer_command.dll")
        );
        assert!(folder_delegate.installed_server_command.is_empty());
    }

    #[test]
    fn current_snapshot_round_trips_folder_delegate_state() {
        let command = r#""C:\Program Files\Example App\mtt-file-manager.exe" "%1""#.to_string();
        let snapshot = DefaultHandlerSnapshot {
            version: 3,
            installed_commands: vec![command.clone()],
            verbs: Vec::new(),
            folder_delegate: Some(FolderDelegateSnapshot {
                classes_root_key_existed: true,
                folder_key_existed: true,
                shell_key_existed: false,
                open_key_existed: false,
                command_key_existed: false,
                default_command: None,
                delegate_execute: None,
                clsid_root_key_existed: true,
                clsid_key_existed: false,
                local_server_key_existed: false,
                local_server: None,
                installed_folder_command: command,
                installed_clsid: CLSID_TEXT.to_string(),
                installed_server_command: r#""C:\Program Files\Example App\mtt-shell-command.exe""#
                    .to_string(),
                legacy_installed_server_path: None,
            }),
        };
        let encoded = serde_json::to_string(&snapshot).unwrap();
        let decoded: DefaultHandlerSnapshot = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, snapshot);
    }
}
