use super::registry::{
    command_for_path, current_executable, delete_key_if_empty, delete_value, key_exists,
    read_value, registry_string_text, write_string, CONTEXT_MENU_ID, CONTEXT_MENU_STATE_KEY,
    REGISTRY_CLASSES_ROOT,
};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

fn context_menu_key_path(class_name: &str) -> String {
    format!("{REGISTRY_CLASSES_ROOT}\\{class_name}\\shell\\{CONTEXT_MENU_ID}")
}

fn context_menu_command_key_path(class_name: &str) -> String {
    format!("{}\\command", context_menu_key_path(class_name))
}

fn remove_context_menu_entry(class_name: &str) -> Result<(), String> {
    let menu_key = context_menu_key_path(class_name);
    let command_key = context_menu_command_key_path(class_name);
    delete_value(&command_key, None)?;
    delete_key_if_empty(&command_key)?;
    for name in [None, Some("Icon"), Some("Position")] {
        delete_value(&menu_key, name)?;
    }
    delete_key_if_empty(&menu_key)
}

fn register_modern_context_menu_package(executable: &Path) {
    let Some(app_directory) = executable.parent() else {
        return;
    };
    let package_path = app_directory.join("mtt-file-manager.identity.msix");
    let script_path = app_directory.join("register_sparse_package.ps1");
    if !package_path.is_file() || !script_path.is_file() {
        return;
    }

    let powershell = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .map(|root| {
            root.join("WindowsPowerShell")
                .join("v1.0")
                .join("powershell.exe")
        })
        .filter(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from("powershell.exe"));
    if let Err(error) = Command::new(powershell)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(script_path)
        .arg("-PackagePath")
        .arg(package_path)
        .arg("-ExternalLocation")
        .arg(app_directory)
        .arg("-PackageName")
        .arg("MTT.FileManager")
        .arg("-PackageVersion")
        .arg(format!("{}.0", env!("CARGO_PKG_VERSION")))
        .creation_flags(0x08000000)
        .spawn()
    {
        log::warn!(
            "[SHELL-INTEGRATION] Could not launch the modern context-menu registration script: {error}"
        );
    }
}

fn set_context_menu_enabled(enabled: bool, label: &str) -> Result<(), String> {
    let executable = current_executable()?;
    let expected_command = command_for_path(&executable);
    let classes = ["Directory", "Drive"];

    if enabled {
        let modern_registration_needed = read_value(CONTEXT_MENU_STATE_KEY, None)?.is_none();
        let mut new_entries = Vec::new();
        for class_name in classes {
            let command_key = context_menu_command_key_path(class_name);
            let menu_key = context_menu_key_path(class_name);
            let current_command = read_value(&command_key, None)?
                .as_ref()
                .and_then(registry_string_text);
            if current_command.as_deref() == Some(&expected_command) {
                continue;
            }
            if key_exists(&menu_key)? {
                return Err(
                    "A context-menu entry already uses the MTT registry identifier; it was left untouched."
                        .to_string(),
                );
            }
            new_entries.push(class_name);
        }

        for class_name in classes {
            let menu_key = context_menu_key_path(class_name);
            let command_key = context_menu_command_key_path(class_name);
            let result = write_string(&menu_key, None, label)
                .and_then(|()| write_string(&menu_key, Some("Icon"), &executable.to_string_lossy()))
                .and_then(|()| write_string(&menu_key, Some("Position"), "Top"))
                .and_then(|()| write_string(&command_key, None, &expected_command));
            if let Err(error) = result {
                for new_class in new_entries {
                    let _ = remove_context_menu_entry(new_class);
                }
                return Err(error);
            }
        }
        if let Err(error) =
            write_string(CONTEXT_MENU_STATE_KEY, None, &executable.to_string_lossy())
        {
            for class_name in classes {
                let _ = remove_context_menu_entry(class_name);
            }
            return Err(error);
        }
        if modern_registration_needed {
            register_modern_context_menu_package(&executable);
        }
        return Ok(());
    }

    let expected_icon = executable.to_string_lossy().into_owned();
    for class_name in classes {
        let command_key = context_menu_command_key_path(class_name);
        let menu_key = context_menu_key_path(class_name);
        let current_command = read_value(&command_key, None)?
            .as_ref()
            .and_then(registry_string_text);
        if current_command.is_some() && current_command.as_deref() != Some(&expected_command) {
            return Err(
                "The MTT context-menu entry changed outside MTT; it was left untouched."
                    .to_string(),
            );
        }
        if current_command.is_none() {
            let current_icon = read_value(&menu_key, Some("Icon"))?
                .as_ref()
                .and_then(registry_string_text);
            let current_label = read_value(&menu_key, None)?;
            let current_position = read_value(&menu_key, Some("Position"))?;
            if current_icon.as_deref() != Some(&expected_icon)
                && (current_label.is_some() || current_position.is_some())
            {
                return Err(
                    "The MTT context-menu entry changed outside MTT; it was left untouched."
                        .to_string(),
                );
            }
        }
    }
    for class_name in classes {
        remove_context_menu_entry(class_name)?;
    }
    delete_value(CONTEXT_MENU_STATE_KEY, None)?;
    delete_key_if_empty(CONTEXT_MENU_STATE_KEY)
}

pub fn set_open_in_mtt_context_menu_enabled(enabled: bool, label: &str) -> Result<(), String> {
    set_context_menu_enabled(enabled, label)
}
