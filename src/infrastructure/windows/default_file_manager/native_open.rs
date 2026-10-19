//! Per-user canonical Open overrides, with reversible value-level backups.
//! Never register these on Folder: that would also intercept virtual folders.

use super::registration::{owns, shell_key, state_key, Registry, RegistryValue};
use std::path::Path;

const VALUES: [(&str, &str, &str, &str); 4] = [
    ("command", "", "OpenCommand", "HadOpenCommand"),
    (
        "command",
        "DelegateExecute",
        "OpenDelegate",
        "HadOpenDelegate",
    ),
    ("", "Icon", "OpenIcon", "HadOpenIcon"),
    ("", "MultiSelectModel", "OpenSelection", "HadOpenSelection"),
];

fn value_key(class: &str, suffix: &str) -> String {
    let verb = format!("{}\\open", shell_key(class));
    if suffix.is_empty() {
        verb
    } else {
        format!("{verb}\\{suffix}")
    }
}

pub(super) fn check_owner(reg: &impl Registry, class: &str, command: &str) -> Result<(), String> {
    if reg
        .read(&state_key(class), "OpenOwner")?
        .is_some_and(|value| !value.eq_ignore_ascii_case(command))
    {
        return Err("The native Open backup belongs to another installation.".into());
    }
    Ok(())
}

pub(super) fn is_active(reg: &impl Registry, class: &str, command: &str) -> bool {
    owns(
        reg.read(&value_key(class, "command"), "")
            .ok()
            .flatten()
            .as_deref(),
        command,
    ) && reg
        .read(&value_key(class, "command"), "DelegateExecute")
        .ok()
        .flatten()
        .as_deref()
        == Some("")
}

pub(super) fn enable(
    reg: &mut impl Registry,
    class: &str,
    command: &str,
    executable: &Path,
) -> Result<(), String> {
    check_owner(reg, class, command)?;
    let state = state_key(class);
    if !owns(
        reg.read(&value_key(class, "command"), "")?.as_deref(),
        command,
    ) {
        for (suffix, name, previous, had) in VALUES {
            let value = reg.read_value(&value_key(class, suffix), name)?;
            reg.write(
                &state,
                previous,
                value.as_ref().map_or("", |value| value.text.as_str()),
            )?;
            reg.write(&state, had, if value.is_some() { "1" } else { "0" })?;
            reg.write(
                &state,
                &format!("Type{previous}"),
                if value.is_some_and(|value| value.expandable) {
                    "expand"
                } else {
                    "string"
                },
            )?;
        }
        reg.write(&state, "OpenOwner", command)?;
    } else if !owns(reg.read(&state, "OpenOwner")?.as_deref(), command) {
        return Err("The native Open backup is missing; the command was left untouched.".into());
    }
    reg.write(&value_key(class, "command"), "", command)?;
    // Disable the inherited COM delegate so the command line is actually used.
    reg.write(&value_key(class, "command"), "DelegateExecute", "")?;
    reg.write(&value_key(class, ""), "Icon", &executable.to_string_lossy())?;
    reg.write(&value_key(class, ""), "MultiSelectModel", "Document")
}

pub(super) fn restore(
    reg: &mut impl Registry,
    class: &str,
    command: &str,
    executable: &Path,
) -> Result<(), String> {
    let state = state_key(class);
    if !owns(reg.read(&state, "OpenOwner")?.as_deref(), command) {
        return Ok(());
    }
    if owns(
        reg.read(&value_key(class, "command"), "")?.as_deref(),
        command,
    ) {
        let icon = executable.to_string_lossy();
        for ((suffix, name, previous, had), expected) in
            VALUES
                .into_iter()
                .zip([command, "", icon.as_ref(), "Document"])
        {
            let key = value_key(class, suffix);
            if !owns(reg.read(&key, name)?.as_deref(), expected) {
                continue;
            }
            match reg.read(&state, had)?.as_deref() {
                Some("1") => {
                    let value = reg
                        .read(&state, previous)?
                        .ok_or("The native Open backup is incomplete.")?;
                    let expandable = match reg.read(&state, &format!("Type{previous}"))?.as_deref()
                    {
                        Some("expand") => true,
                        Some("string") => false,
                        _ => return Err("The native Open backup type is missing.".into()),
                    };
                    reg.write_value(
                        &key,
                        name,
                        &RegistryValue {
                            text: value,
                            expandable,
                        },
                    )?;
                }
                Some("0") => reg.delete(&key, name)?,
                _ => return Err("The native Open backup is incomplete.".into()),
            }
        }
        reg.delete_key_if_empty(&value_key(class, "command"))?;
        reg.delete_key_if_empty(&value_key(class, ""))?;
    }
    for (_, _, previous, had) in VALUES {
        reg.delete(&state, previous)?;
        reg.delete(&state, had)?;
        reg.delete(&state, &format!("Type{previous}"))?;
    }
    reg.delete(&state, "OpenOwner")?;
    reg.delete_key_if_empty(&state)
}
