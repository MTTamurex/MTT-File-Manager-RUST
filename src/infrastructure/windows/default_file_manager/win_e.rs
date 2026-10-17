use super::{owns, win_e_command, Registry, WIN_E_COMMAND_KEY};
use std::path::Path;

const STATE_KEY: &str = "Software\\MTT-File-Manager\\ShellIntegration\\WinE";

pub(super) fn enable(reg: &mut impl Registry, executable: &Path) -> Result<(), String> {
    let command = win_e_command(executable);
    if reg
        .read(STATE_KEY, "Command")?
        .is_some_and(|owner| !owns(Some(&owner), &command))
    {
        return Err("The Win+E backup belongs to another installation.".into());
    }
    let current = reg.read(WIN_E_COMMAND_KEY, "")?;
    if !owns(current.as_deref(), &command) {
        let delegate = reg.read(WIN_E_COMMAND_KEY, "DelegateExecute")?;
        for (previous, had, value) in [
            ("PreviousDefault", "HadDefault", current.as_deref()),
            (
                "PreviousDelegateExecute",
                "HadDelegateExecute",
                delegate.as_deref(),
            ),
        ] {
            reg.write(STATE_KEY, previous, value.unwrap_or(""))?;
            reg.write(STATE_KEY, had, if value.is_some() { "1" } else { "0" })?;
        }
        reg.write(STATE_KEY, "Command", &command)?;
    }
    // Existing installations without a backup cannot reconstruct the old
    // settings. Do not snapshot our own command as their previous default.
    reg.write(WIN_E_COMMAND_KEY, "", &command)?;
    reg.write(WIN_E_COMMAND_KEY, "DelegateExecute", "")
}

pub(super) fn restore(reg: &mut impl Registry, executable: &Path) -> Result<(), String> {
    let command = win_e_command(executable);
    let backup_owner = reg.read(STATE_KEY, "Command")?;
    if backup_owner
        .as_deref()
        .is_some_and(|owner| !owns(Some(owner), &command))
    {
        return Ok(());
    }
    if owns(reg.read(WIN_E_COMMAND_KEY, "")?.as_deref(), &command) {
        restore_value(
            reg,
            "",
            "PreviousDefault",
            "HadDefault",
            backup_owner.is_some(),
        )?;
        // Preserve a delegate changed by another application after Enable.
        if reg.read(WIN_E_COMMAND_KEY, "DelegateExecute")?.as_deref() == Some("") {
            restore_value(
                reg,
                "DelegateExecute",
                "PreviousDelegateExecute",
                "HadDelegateExecute",
                backup_owner.is_some(),
            )?;
        }
    }
    if backup_owner.is_some() {
        for name in [
            "PreviousDefault",
            "HadDefault",
            "PreviousDelegateExecute",
            "HadDelegateExecute",
            "Command",
        ] {
            reg.delete(STATE_KEY, name)?;
        }
        reg.delete_key_if_empty(STATE_KEY)?;
    }
    Ok(())
}

fn restore_value(
    reg: &mut impl Registry,
    name: &str,
    previous: &str,
    had: &str,
    has_backup: bool,
) -> Result<(), String> {
    if has_backup {
        match reg.read(STATE_KEY, had)?.as_deref() {
            Some("1") => {
                let value = reg
                    .read(STATE_KEY, previous)?
                    .ok_or("The previous Win+E setting is missing.")?;
                return reg.write(WIN_E_COMMAND_KEY, name, &value);
            }
            Some("0") => {}
            _ => return Err("The Win+E backup is incomplete.".into()),
        }
    }
    reg.delete(WIN_E_COMMAND_KEY, name)
}
