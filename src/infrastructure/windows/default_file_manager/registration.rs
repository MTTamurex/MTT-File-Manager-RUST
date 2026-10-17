use std::path::Path;

#[path = "win_e.rs"]
mod win_e;

pub(super) const VERB: &str = "MTT.FileManager.Default";
pub(super) const CLASSES: [&str; 2] = ["Directory", "Drive"];
pub(super) const NETWORK_LOCATION_CLASS: &str = "CLSID\\{0AFACED1-E828-11D1-9187-B532F1E9575D}";
pub(super) const LEGACY_KEYS: [&str; 2] = [
    "Software\\Classes\\Folder\\shell\\open\\command",
    "Software\\Classes\\Folder\\shell\\explore\\command",
];
pub(super) const WIN_E_COMMAND_KEY: &str = concat!(
    "Software\\Classes\\CLSID\\{52205fd8-5dfb-447d-801a-d0b52f2e83e1}",
    "\\shell\\opennewwindow\\command"
);

pub(super) trait Registry {
    fn read(&self, path: &str, name: &str) -> Result<Option<String>, String>;
    fn write(&mut self, path: &str, name: &str, value: &str) -> Result<(), String>;
    fn delete(&mut self, path: &str, name: &str) -> Result<(), String>;
    fn delete_key_if_empty(&mut self, path: &str) -> Result<(), String>;
}

#[derive(Clone, Copy)]
pub(super) enum RegistryAction {
    UpgradeFolderCommands,
    EnableFolders,
    RestoreFolders,
    EnableWinE,
    RestoreWinE,
}

pub(super) fn folder_command(executable: &Path) -> String {
    // A root ends in a backslash, which would escape the closing quote in argv.
    // Appending a directory component keeps it away from the quote; the receiver
    // removes the lexical `.` component without filesystem access.
    format!(
        "\"{}\" --open-folder \"%1\\.\"",
        executable.to_string_lossy()
    )
}

pub(super) fn legacy_folder_command(executable: &Path) -> String {
    format!("\"{}\" --open-folder \"%1\"", executable.to_string_lossy())
}

pub(super) fn needs_folder_command_upgrade(reg: &impl Registry, executable: &Path) -> bool {
    let old = legacy_folder_command(executable);
    CLASSES
        .iter()
        .map(|class| format!("{}\\{VERB}\\command", shell_key(class)))
        .chain(LEGACY_KEYS.iter().map(|key| key.to_string()))
        .any(|key| owns(reg.read(&key, "").ok().flatten().as_deref(), &old))
}

fn upgrade_folder_commands(reg: &mut impl Registry, executable: &Path) -> Result<(), String> {
    let old = legacy_folder_command(executable);
    let new = folder_command(executable);
    for key in CLASSES
        .iter()
        .map(|class| format!("{}\\{VERB}\\command", shell_key(class)))
        .chain(LEGACY_KEYS.iter().map(|key| key.to_string()))
    {
        if owns(reg.read(&key, "")?.as_deref(), &old) {
            reg.write(&key, "", &new)?;
        }
    }
    Ok(())
}

pub(super) fn network_location_command(executable: &Path) -> String {
    format!(
        "\"{}\" --open-network-location \"%1\"",
        executable.to_string_lossy()
    )
}

pub(super) fn needs_network_location_upgrade(reg: &impl Registry, executable: &Path) -> bool {
    CLASSES
        .iter()
        .all(|class| class_is_active(reg, class, &folder_command(executable)))
        && matches!(
            reg.read(
                &format!("{}\\{VERB}\\command", shell_key(NETWORK_LOCATION_CLASS)),
                ""
            ),
            Ok(None)
        )
}

pub(super) fn win_e_command(executable: &Path) -> String {
    format!("\"{}\"", executable.to_string_lossy())
}

pub(super) fn shell_key(class: &str) -> String {
    format!("Software\\Classes\\{class}\\shell")
}

pub(super) fn owns(value: Option<&str>, expected: &str) -> bool {
    value.is_some_and(|value| value.eq_ignore_ascii_case(expected))
}

pub(super) fn class_is_active(reg: &impl Registry, class: &str, command: &str) -> bool {
    let shell = shell_key(class);
    owns(reg.read(&shell, "").ok().flatten().as_deref(), VERB)
        && owns(
            reg.read(&format!("{shell}\\{VERB}\\command"), "")
                .ok()
                .flatten()
                .as_deref(),
            command,
        )
}

pub(super) fn apply(
    reg: &mut impl Registry,
    action: RegistryAction,
    executable: &Path,
) -> Result<(), String> {
    super::registry_transaction::run(reg, |reg| apply_action(reg, action, executable))
}

fn apply_action(
    reg: &mut impl Registry,
    action: RegistryAction,
    executable: &Path,
) -> Result<(), String> {
    let command = folder_command(executable);
    if matches!(
        action,
        RegistryAction::UpgradeFolderCommands
            | RegistryAction::EnableFolders
            | RegistryAction::RestoreFolders
    ) {
        upgrade_folder_commands(reg, executable)?;
    }
    match action {
        RegistryAction::UpgradeFolderCommands => {}
        RegistryAction::EnableFolders => {
            // Check every class before changing any existing association.
            for class in CLASSES {
                check_class_owner(reg, class, &command)?;
            }
            check_class_owner(
                reg,
                NETWORK_LOCATION_CLASS,
                &network_location_command(executable),
            )?;
            for class in CLASSES {
                enable_class(reg, class, &command)?;
            }
            enable_class(
                reg,
                NETWORK_LOCATION_CLASS,
                &network_location_command(executable),
            )?;
            remove_legacy(reg, &command)?;
        }
        RegistryAction::RestoreFolders => {
            for class in CLASSES {
                restore_class(reg, class, &command)?;
            }
            restore_class(
                reg,
                NETWORK_LOCATION_CLASS,
                &network_location_command(executable),
            )?;
            remove_legacy(reg, &command)?;
        }
        RegistryAction::EnableWinE => win_e::enable(reg, executable)?,
        RegistryAction::RestoreWinE => win_e::restore(reg, executable)?,
    }
    Ok(())
}

fn state_key(class: &str) -> String {
    format!("Software\\MTT-File-Manager\\ShellIntegration\\DefaultFolders\\{class}")
}

fn check_class_owner(reg: &impl Registry, class: &str, command: &str) -> Result<(), String> {
    let command_key = format!("{}\\{VERB}\\command", shell_key(class));
    if reg
        .read(&command_key, "")?
        .is_some_and(|value| !value.eq_ignore_ascii_case(command))
    {
        return Err("The MTT folder verb belongs to another installation.".into());
    }
    Ok(())
}

fn enable_class(reg: &mut impl Registry, class: &str, command: &str) -> Result<(), String> {
    check_class_owner(reg, class, command)?;
    let shell = shell_key(class);
    let verb = format!("{shell}\\{VERB}");
    let command_key = format!("{verb}\\command");
    let state = state_key(class);
    let current = reg.read(&shell, "")?;
    if !owns(current.as_deref(), VERB) {
        // Save before selecting our verb, including whether the value was absent.
        // Re-enabling an already selected verb must not overwrite this backup.
        reg.write(&state, "PreviousDefault", current.as_deref().unwrap_or(""))?;
        reg.write(
            &state,
            "HadDefault",
            if current.is_some() { "1" } else { "0" },
        )?;
    }
    reg.write(&verb, "", "Open in MTT")?;
    reg.write(&verb, "MultiSelectModel", "Document")?;
    reg.write(&command_key, "", command)?;
    reg.write(&shell, "", VERB)
}

fn restore_class(reg: &mut impl Registry, class: &str, command: &str) -> Result<(), String> {
    let shell = shell_key(class);
    let verb = format!("{shell}\\{VERB}");
    let command_key = format!("{verb}\\command");
    if !owns(reg.read(&command_key, "")?.as_deref(), command) {
        return Ok(());
    }
    let state = state_key(class);
    if owns(reg.read(&shell, "")?.as_deref(), VERB) {
        if reg.read(&state, "HadDefault")?.as_deref() == Some("1") {
            let previous = reg
                .read(&state, "PreviousDefault")?
                .ok_or("The previous folder association is missing.")?;
            reg.write(&shell, "", &previous)?;
        } else {
            reg.delete(&shell, "")?;
        }
    }
    reg.delete(&command_key, "")?;
    reg.delete(&verb, "")?;
    reg.delete(&verb, "MultiSelectModel")?;
    reg.delete(&state, "PreviousDefault")?;
    reg.delete(&state, "HadDefault")?;
    reg.delete_key_if_empty(&command_key)?;
    reg.delete_key_if_empty(&verb)?;
    reg.delete_key_if_empty(&state)
}

fn remove_legacy(reg: &mut impl Registry, command: &str) -> Result<(), String> {
    for key in LEGACY_KEYS {
        if owns(reg.read(key, "")?.as_deref(), command) {
            // Removing our overrides exposes the native machine-wide Shell handlers.
            reg.delete(key, "")?;
            if reg.read(key, "DelegateExecute")?.as_deref() == Some("") {
                reg.delete(key, "DelegateExecute")?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "registration_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "shell_command_tests.rs"]
mod shell_command_tests;
