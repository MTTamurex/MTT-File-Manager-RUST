//! Per-user registration for the Explorer "Open in MTT" folder verb.

use std::path::{Path, PathBuf};

mod registry;
use registry::{
    delete_key_if_empty, delete_value, key_exists, notify_shell_association_changed, read_string,
    write_string,
};

const VERB_KEY: &str = "Software\\Classes\\Directory\\shell\\MTT.FileManager.OpenInMTT";
const COMMAND_KEY: &str = "Software\\Classes\\Directory\\shell\\MTT.FileManager.OpenInMTT\\command";
const STATE_KEY: &str = "Software\\MTT-File-Manager\\ShellIntegration\\ContextMenu";
const VERB_POSITION: &str = "Top";

pub fn is_open_in_mtt_context_menu_enabled() -> bool {
    let Ok(executable) = std::env::current_exe() else {
        return false;
    };
    let expected_command = command_for_path(&executable);
    let expected_path = executable.to_string_lossy().into_owned();
    let command = read_string(COMMAND_KEY, None).ok().flatten();
    let marker = read_string(STATE_KEY, None).ok().flatten();
    let command_matches = command
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case(&expected_command));
    let marker_matches = marker
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case(&expected_path));
    command_matches || (command.is_none() && marker_matches)
}

pub fn set_open_in_mtt_context_menu_enabled(enabled: bool, label: &str) -> Result<(), String> {
    let executable = current_executable()?;
    let expected_path = executable.to_string_lossy().into_owned();
    let expected_command = command_for_path(&executable);
    let current_command = read_string(COMMAND_KEY, None)?;
    let current_marker = read_string(STATE_KEY, None)?;
    let command_already_owned = current_command
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case(&expected_command));

    if enabled {
        if current_command
            .as_deref()
            .is_some_and(|value| !value.eq_ignore_ascii_case(&expected_command))
        {
            return Err("A different command already owns the MTT context-menu key.".to_string());
        }
        if current_marker
            .as_deref()
            .is_some_and(|value| !value.eq_ignore_ascii_case(&expected_path))
        {
            return Err("The MTT context-menu state belongs to another installation.".to_string());
        }
        if !command_already_owned && current_marker.is_none() && key_exists(VERB_KEY)? {
            return Err(
                "A context-menu entry already uses the MTT registry identifier.".to_string(),
            );
        }

        let result = delete_value(VERB_KEY, Some("MultiSelectModel"))
            .and_then(|()| write_string(VERB_KEY, None, label))
            .and_then(|()| write_string(VERB_KEY, Some("Icon"), &expected_path))
            .and_then(|()| write_string(VERB_KEY, Some("Position"), VERB_POSITION))
            .and_then(|()| write_string(COMMAND_KEY, None, &expected_command))
            .and_then(|()| write_string(STATE_KEY, None, &expected_path));
        if let Err(error) = result {
            if !command_already_owned {
                let _ = remove_context_menu_entry();
                let _ = delete_owned_state(&expected_path);
            }
            return Err(error);
        }
        notify_shell_association_changed();
        return Ok(());
    }

    if current_command
        .as_deref()
        .is_some_and(|value| !value.eq_ignore_ascii_case(&expected_command))
    {
        return Err("The MTT context-menu command changed and was left untouched.".to_string());
    }
    if current_marker
        .as_deref()
        .is_some_and(|value| !value.eq_ignore_ascii_case(&expected_path))
    {
        return Err("The MTT context-menu state changed and was left untouched.".to_string());
    }

    if current_command.is_some() || current_marker.is_some() {
        remove_context_menu_entry()?;
        delete_owned_state(&expected_path)?;
        notify_shell_association_changed();
    }
    Ok(())
}

fn current_executable() -> Result<PathBuf, String> {
    std::env::current_exe().map_err(|error| format!("Could not locate the MTT executable: {error}"))
}

fn command_for_path(executable: &Path) -> String {
    format!("\"{}\" --open-folder \"%1\"", executable.to_string_lossy())
}

fn remove_context_menu_entry() -> Result<(), String> {
    let command_key = format!("{VERB_KEY}\\command");
    delete_value(&command_key, None)?;
    delete_key_if_empty(&command_key)?;
    for name in [
        None,
        Some("Icon"),
        Some("Position"),
        Some("MultiSelectModel"),
    ] {
        delete_value(VERB_KEY, name)?;
    }
    delete_key_if_empty(VERB_KEY)
}

fn delete_owned_state(expected_path: &str) -> Result<(), String> {
    if read_string(STATE_KEY, None)?
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case(expected_path))
    {
        delete_value(STATE_KEY, None)?;
        delete_key_if_empty(STATE_KEY)?;
        delete_key_if_empty("Software\\MTT-File-Manager\\ShellIntegration")?;
        delete_key_if_empty("Software\\MTT-File-Manager")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{command_for_path, VERB_POSITION};
    use std::path::Path;

    #[test]
    fn context_verb_quotes_the_executable_and_folder_argument() {
        assert_eq!(
            command_for_path(Path::new(r"C:\Program Files\Example\mtt-file-manager.exe")),
            r#""C:\Program Files\Example\mtt-file-manager.exe" --open-folder "%1""#
        );
    }

    #[test]
    fn static_context_verb_requests_top_position() {
        assert_eq!(VERB_POSITION, "Top");
    }
}
