//! Remove deprecated extra menu verbs without touching another installation.

use super::registration::{
    folder_command, legacy_folder_command, network_location_command, owns, shell_key, Registry,
    CLASSES, NETWORK_LOCATION_CLASS, VERB,
};
use std::path::Path;

const OLD_VERB: &str = "Software\\Classes\\Directory\\shell\\MTT.FileManager.OpenInMTT";
const OLD_COMMAND: &str = "Software\\Classes\\Directory\\shell\\MTT.FileManager.OpenInMTT\\command";
const OLD_STATE: &str = "Software\\MTT-File-Manager\\ShellIntegration\\ContextMenu";

pub(super) fn has_owned_entries(reg: &impl Registry, executable: &Path) -> bool {
    CLASSES
        .into_iter()
        .chain([NETWORK_LOCATION_CLASS])
        .any(|class| {
            let key = format!("{}\\{VERB}\\command", shell_key(class));
            let command = if class == NETWORK_LOCATION_CLASS {
                network_location_command(executable)
            } else {
                folder_command(executable)
            };
            let value = reg.read(&key, "").ok().flatten();
            owns(value.as_deref(), &command)
                || (class != NETWORK_LOCATION_CLASS
                    && owns(value.as_deref(), &legacy_folder_command(executable)))
        })
        || (owns(
            reg.read(OLD_COMMAND, "").ok().flatten().as_deref(),
            &legacy_folder_command(executable),
        ) && reg.read(OLD_STATE, "").is_ok_and(|marker| {
            marker.is_none() || owns(marker.as_deref(), &executable.to_string_lossy())
        }))
}

pub(super) fn remove_class(
    reg: &mut impl Registry,
    class: &str,
    command: &str,
) -> Result<(), String> {
    let verb = format!("{}\\{VERB}", shell_key(class));
    let key = format!("{verb}\\command");
    if owns(reg.read(&key, "")?.as_deref(), command) {
        remove_entry(reg, &verb, &key)?;
    }
    Ok(())
}

fn remove_entry(reg: &mut impl Registry, verb: &str, command_key: &str) -> Result<(), String> {
    reg.delete(command_key, "")?;
    for name in [
        "",
        "Icon",
        "Position",
        "MultiSelectModel",
        "ProgrammaticAccessOnly",
    ] {
        reg.delete(verb, name)?;
    }
    reg.delete_key_if_empty(command_key)?;
    reg.delete_key_if_empty(verb)
}

pub(super) fn remove_legacy(reg: &mut impl Registry, executable: &Path) -> Result<(), String> {
    let command = reg.read(OLD_COMMAND, "")?;
    let marker = reg.read(OLD_STATE, "")?;
    let expected_path = executable.to_string_lossy();
    if command
        .as_deref()
        .is_some_and(|value| !value.eq_ignore_ascii_case(&legacy_folder_command(executable)))
        || marker
            .as_deref()
            .is_some_and(|value| !value.eq_ignore_ascii_case(&expected_path))
        || (command.is_none() && marker.is_none())
    {
        return Ok(());
    }
    remove_entry(reg, OLD_VERB, OLD_COMMAND)?;
    if owns(marker.as_deref(), &expected_path) {
        reg.delete(OLD_STATE, "")?;
        reg.delete_key_if_empty(OLD_STATE)?;
    }
    Ok(())
}
