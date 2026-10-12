use std::path::Path;

const USER_CLASSES: &str = "HKEY_CURRENT_USER\\Software\\Classes";
const FOLDER_OPEN_COMMAND_KEY: &str =
    "HKEY_CURRENT_USER\\Software\\Classes\\Folder\\shell\\open\\command";
const FOLDER_EXPLORE_COMMAND_KEY: &str =
    "HKEY_CURRENT_USER\\Software\\Classes\\Folder\\shell\\explore\\command";
const WIN_E_COMMAND_KEY: &str = concat!(
    "HKEY_CURRENT_USER\\Software\\Classes\\CLSID\\{52205fd8-5dfb-447d-801a-d0b52f2e83e1}",
    "\\shell\\opennewwindow\\command"
);
const EXPLORER_DELEGATE_EXECUTE: &str = "{11dbb47c-a525-400b-9e80-a54615a090c0}";

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

pub(super) fn registry_script(
    action: RegistryAction,
    executable: &Path,
    status: super::RegistrationStatus,
) -> String {
    let mut sections = Vec::new();
    match action {
        RegistryAction::EnableFolders => {
            let command = folder_command(executable);
            sections.push(format!(
                "[{USER_CLASSES}\\Folder\\shell\\open\\command]\n@={}\n\"DelegateExecute\"=\"\"",
                registry_string(&command)
            ));
            sections.push(format!(
                "[{USER_CLASSES}\\Folder\\shell\\explore\\command]\n@={}\n\"DelegateExecute\"=\"\"",
                registry_string(&command)
            ));
        }
        RegistryAction::RestoreFolders => {
            if status.folder_open {
                sections.push(format!(
                    "[{USER_CLASSES}\\Folder\\shell\\open]\n\"MultiSelectModel\"=\"Document\"\n\n[{FOLDER_OPEN_COMMAND_KEY}]\n@={}\n\"DelegateExecute\"=\"{EXPLORER_DELEGATE_EXECUTE}\"",
                    registry_expand_string("%SystemRoot%\\Explorer.exe")
                ));
            }
            if status.folder_explore {
                sections.push(format!(
                    "[{USER_CLASSES}\\Folder\\shell\\explore]\n\"LaunchExplorerFlags\"=dword:00000018\n\"MultiSelectModel\"=\"Document\"\n\"ProgrammaticAccessOnly\"=\"\"\n\n[{FOLDER_EXPLORE_COMMAND_KEY}]\n@=-\n\"DelegateExecute\"=\"{EXPLORER_DELEGATE_EXECUTE}\""
                ));
            }
        }
        RegistryAction::EnableWinE => {
            let win_e = win_e_command(executable);
            sections.push(format!(
                "[{WIN_E_COMMAND_KEY}]\n@={}\n\"DelegateExecute\"=\"\"",
                registry_string(&win_e)
            ));
        }
        RegistryAction::RestoreWinE => {
            if status.win_e {
                sections.push(format!("[{WIN_E_COMMAND_KEY}]\n@=-\n\"DelegateExecute\"=-"));
            }
        }
    }

    format!(
        "Windows Registry Editor Version 5.00\n\n{}\n",
        sections.join("\n\n")
    )
}

fn registry_string(value: &str) -> String {
    registry_string_with_type(value, 1)
}

fn registry_expand_string(value: &str) -> String {
    registry_string_with_type(value, 2)
}

fn registry_string_with_type(value: &str, registry_type: u8) -> String {
    let bytes: Vec<u8> = value
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect();
    let lines = bytes
        .chunks(16)
        .map(|chunk| {
            chunk
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<Vec<_>>()
                .join(",")
        })
        .collect::<Vec<_>>()
        .join(",\\\n  ");
    format!("hex({registry_type:x}):{lines}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_command_quotes_executable_and_forwarded_folder() {
        let executable = Path::new(r"C:\Program Files\Example\mtt-file-manager.exe");
        assert_eq!(
            folder_command(executable),
            r#""C:\Program Files\Example\mtt-file-manager.exe" --open-folder "%1""#
        );
    }

    #[test]
    fn registry_string_encoding_is_utf16le_and_null_terminated() {
        assert_eq!(registry_string_with_type("A", 1), "hex(1):41,00,00,00");
        assert!(registry_string_with_type("é", 1).starts_with("hex(1):e9,00"));
    }

    #[test]
    fn folder_default_script_only_targets_current_user_folder_classes() {
        let executable = Path::new(r"C:\Program Files\Example\mtt-file-manager.exe");
        let script = registry_script(
            RegistryAction::EnableFolders,
            executable,
            super::super::RegistrationStatus::default(),
        );

        assert!(
            script.contains("[HKEY_CURRENT_USER\\Software\\Classes\\Folder\\shell\\open\\command]")
        );
        assert!(script
            .contains("[HKEY_CURRENT_USER\\Software\\Classes\\Folder\\shell\\explore\\command]"));
        assert!(!script.contains("opennewwindow"));
        assert!(!script.contains("HKEY_LOCAL_MACHINE"));
    }

    #[test]
    fn win_e_enable_script_only_targets_the_current_user_opennewwindow_command() {
        let executable = Path::new(r"C:\Program Files\Example\mtt-file-manager.exe");
        let script = registry_script(
            RegistryAction::EnableWinE,
            executable,
            super::super::RegistrationStatus::default(),
        );

        assert!(script.contains("opennewwindow\\command"));
        assert!(!script.contains("Folder\\shell\\open\\command"));
        assert!(!script.contains("HKEY_LOCAL_MACHINE"));
    }

    #[test]
    fn restore_script_changes_only_values_still_owned_by_mtt() {
        let executable = Path::new(r"C:\Program Files\Example\mtt-file-manager.exe");
        let script = registry_script(
            RegistryAction::RestoreFolders,
            executable,
            super::super::RegistrationStatus {
                folder_open: true,
                folder_explore: false,
                context_menu: false,
                win_e: true,
            },
        );

        assert!(script.contains("Folder\\shell\\open\\command"));
        assert!(script.contains(&registry_expand_string("%SystemRoot%\\Explorer.exe")));
        assert!(!script.contains("opennewwindow"));
        assert!(!script.contains("Folder\\shell\\explore\\command"));
    }

    #[test]
    fn win_e_restore_script_only_removes_the_mtt_opennewwindow_override() {
        let executable = Path::new(r"C:\Program Files\Example\mtt-file-manager.exe");
        let script = registry_script(
            RegistryAction::RestoreWinE,
            executable,
            super::super::RegistrationStatus {
                win_e: true,
                ..super::super::RegistrationStatus::default()
            },
        );

        assert!(script.contains("opennewwindow\\command"));
        assert!(script.contains("@=-"));
        assert!(!script.contains("Folder\\shell\\open\\command"));
    }
}
