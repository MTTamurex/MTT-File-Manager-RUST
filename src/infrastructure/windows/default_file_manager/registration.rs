use std::path::Path;

pub(super) const VERB: &str = "MTT.FileManager.Default";
pub(super) const CLASSES: [&str; 2] = ["Directory", "Drive"];
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
    EnableFolders,
    RestoreFolders,
    EnableWinE,
    RestoreWinE,
}

pub(super) fn folder_command(executable: &Path) -> String {
    format!("\"{}\" --open-folder \"%1\"", executable.to_string_lossy())
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
    let command = folder_command(executable);
    match action {
        RegistryAction::EnableFolders => {
            for class in CLASSES {
                enable_class(reg, class, &command)?;
            }
            remove_legacy(reg, &command)?;
        }
        RegistryAction::RestoreFolders => {
            for class in CLASSES {
                restore_class(reg, class, &command)?;
            }
            remove_legacy(reg, &command)?;
        }
        RegistryAction::EnableWinE => {
            reg.write(WIN_E_COMMAND_KEY, "", &win_e_command(executable))?;
            reg.write(WIN_E_COMMAND_KEY, "DelegateExecute", "")?;
        }
        RegistryAction::RestoreWinE => {
            if owns(
                reg.read(WIN_E_COMMAND_KEY, "")?.as_deref(),
                &win_e_command(executable),
            ) {
                reg.delete(WIN_E_COMMAND_KEY, "")?;
                if reg.read(WIN_E_COMMAND_KEY, "DelegateExecute")?.as_deref() == Some("") {
                    reg.delete(WIN_E_COMMAND_KEY, "DelegateExecute")?;
                }
            }
        }
    }
    Ok(())
}

fn state_key(class: &str) -> String {
    format!("Software\\MTT-File-Manager\\ShellIntegration\\DefaultFolders\\{class}")
}

fn enable_class(reg: &mut impl Registry, class: &str, command: &str) -> Result<(), String> {
    let shell = shell_key(class);
    let verb = format!("{shell}\\{VERB}");
    let command_key = format!("{verb}\\command");
    if reg
        .read(&command_key, "")?
        .is_some_and(|value| !value.eq_ignore_ascii_case(command))
    {
        return Err("The MTT folder verb belongs to another installation.".into());
    }
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
