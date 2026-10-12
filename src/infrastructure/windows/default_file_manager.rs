//! Registers MTT as the per-user folder handler and context-menu command, with optional Win+E handling.
//!
//! The operation is explicit: the user starts it from Settings, Windows
//! presents a UAC prompt, and regedit imports a narrowly scoped .reg file.
//! Registry values are queried again after import to verify the result.

mod elevation;
mod registry_access;
mod registry_script;

use elevation::import_registry_script;
use registry_access::read_current_user_string;
use registry_script::{folder_command, registry_script, win_e_command, RegistryAction};

const FOLDER_OPEN_COMMAND_KEY: &str = "Software\\Classes\\Folder\\shell\\open\\command";
const FOLDER_EXPLORE_COMMAND_KEY: &str = "Software\\Classes\\Folder\\shell\\explore\\command";
const WIN_E_COMMAND_KEY: &str = concat!(
    "Software\\Classes\\CLSID\\{52205fd8-5dfb-447d-801a-d0b52f2e83e1}",
    "\\shell\\opennewwindow\\command"
);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RegistrationStatus {
    pub folder_open: bool,
    pub folder_explore: bool,
    pub context_menu: bool,
    pub win_e: bool,
}

impl RegistrationStatus {
    /// Whether any part of the combined default-folder integration is active.
    pub const fn is_active(self) -> bool {
        self.folder_open || self.folder_explore || self.context_menu
    }

    pub const fn has_folder_handlers(self) -> bool {
        self.folder_open || self.folder_explore
    }

    /// Whether folder opening and the selected-folder verb are all registered.
    pub const fn is_complete(self) -> bool {
        self.folder_open && self.folder_explore && self.context_menu
    }

    pub const fn folders_are_complete(self) -> bool {
        self.folder_open && self.folder_explore
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegistryActionError {
    Cancelled,
    Failed,
}

/// Read the folder, context-menu, and Win+E registrations owned by MTT for the current user.
pub fn registration_status() -> RegistrationStatus {
    let Ok(executable) = std::env::current_exe() else {
        return RegistrationStatus::default();
    };

    let folder_command = folder_command(&executable);
    let win_e_command = win_e_command(&executable);
    RegistrationStatus {
        folder_open: read_current_user_string(FOLDER_OPEN_COMMAND_KEY, "")
            .is_some_and(|value| value.eq_ignore_ascii_case(&folder_command)),
        folder_explore: read_current_user_string(FOLDER_EXPLORE_COMMAND_KEY, "")
            .is_some_and(|value| value.eq_ignore_ascii_case(&folder_command)),
        context_menu:
            crate::infrastructure::windows::shell_context_menu::is_open_in_mtt_context_menu_enabled(
            ),
        win_e: read_current_user_string(WIN_E_COMMAND_KEY, "")
            .is_some_and(|value| value.eq_ignore_ascii_case(&win_e_command)),
    }
}

/// Register MTT for folder opening and add its folder context-menu command, without changing Win+E.
pub fn set_as_default(
    owner_window: Option<isize>,
    context_menu_label: &str,
) -> Result<RegistrationStatus, RegistryActionError> {
    let context_menu_was_enabled =
        crate::infrastructure::windows::shell_context_menu::is_open_in_mtt_context_menu_enabled();
    if let Err(error) =
        crate::infrastructure::windows::shell_context_menu::set_open_in_mtt_context_menu_enabled(
            true,
            context_menu_label,
        )
    {
        log::error!("[DEFAULT-FILE-MANAGER] Could not add Open in MTT: {error}");
        return Err(RegistryActionError::Failed);
    }
    if let Err(error) = apply_action(RegistryAction::EnableFolders, owner_window) {
        if !context_menu_was_enabled {
            let _ = crate::infrastructure::windows::shell_context_menu::
                set_open_in_mtt_context_menu_enabled(false, context_menu_label);
        }
        return Err(error);
    }

    let status = registration_status();
    if !status.is_complete() {
        log::error!(
            "[DEFAULT-FILE-MANAGER] Combined registration verification failed: folder_open={} folder_explore={} context_menu={}",
            status.folder_open,
            status.folder_explore,
            status.context_menu
        );
        return Err(RegistryActionError::Failed);
    }
    Ok(status)
}

/// Restore Windows Explorer's folder-open handlers and remove Open in MTT, without changing Win+E.
pub fn restore_explorer(
    owner_window: Option<isize>,
    context_menu_label: &str,
) -> Result<RegistrationStatus, RegistryActionError> {
    apply_action(RegistryAction::RestoreFolders, owner_window)?;
    if let Err(error) =
        crate::infrastructure::windows::shell_context_menu::set_open_in_mtt_context_menu_enabled(
            false,
            context_menu_label,
        )
    {
        log::error!("[DEFAULT-FILE-MANAGER] Could not remove Open in MTT: {error}");
        return Err(RegistryActionError::Failed);
    }

    let status = registration_status();
    if status.is_active() {
        log::error!("[DEFAULT-FILE-MANAGER] Combined Explorer restore verification failed");
        return Err(RegistryActionError::Failed);
    }
    Ok(status)
}

/// Independently enable or restore the Win+E / Explorer launch route.
pub fn set_win_e_enabled(
    enabled: bool,
    owner_window: Option<isize>,
) -> Result<RegistrationStatus, RegistryActionError> {
    let action = if enabled {
        RegistryAction::EnableWinE
    } else {
        RegistryAction::RestoreWinE
    };
    apply_action(action, owner_window)
}

fn apply_action(
    action: RegistryAction,
    owner_window: Option<isize>,
) -> Result<RegistrationStatus, RegistryActionError> {
    let executable = std::env::current_exe().map_err(|_| RegistryActionError::Failed)?;
    let current_status = registration_status();

    if matches!(action, RegistryAction::RestoreFolders) && !current_status.has_folder_handlers() {
        return Ok(current_status);
    }
    if matches!(action, RegistryAction::RestoreWinE) && !current_status.win_e {
        return Ok(current_status);
    }

    let script = registry_script(action, &executable, current_status);
    import_registry_script(&script, owner_window)?;

    let status = registration_status();
    match action {
        RegistryAction::EnableFolders if !status.folders_are_complete() => {
            log::error!(
                "[DEFAULT-FILE-MANAGER] Registry import verification failed: folder_open={} folder_explore={}",
                status.folder_open,
                status.folder_explore
            );
            Err(RegistryActionError::Failed)
        }
        RegistryAction::RestoreFolders if status.has_folder_handlers() => {
            log::error!(
                "[DEFAULT-FILE-MANAGER] Explorer restore verification failed: folder_open={} folder_explore={}",
                status.folder_open,
                status.folder_explore
            );
            Err(RegistryActionError::Failed)
        }
        RegistryAction::EnableWinE if !status.win_e => {
            log::error!("[WIN-E-INTEGRATION] Registry import verification failed");
            Err(RegistryActionError::Failed)
        }
        RegistryAction::RestoreWinE if status.win_e => {
            log::error!("[WIN-E-INTEGRATION] Explorer restore verification failed");
            Err(RegistryActionError::Failed)
        }
        _ => Ok(status),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_status_distinguishes_partial_registration() {
        let status = RegistrationStatus {
            folder_open: true,
            folder_explore: false,
            context_menu: false,
            win_e: true,
        };

        assert!(status.is_active());
        assert!(!status.is_complete());
    }

    #[test]
    fn win_e_registration_is_independent_of_default_folder_handlers() {
        let status = RegistrationStatus {
            folder_open: false,
            folder_explore: false,
            context_menu: false,
            win_e: true,
        };

        assert!(!status.is_active());
        assert!(!status.is_complete());
    }

    #[test]
    fn context_menu_is_part_of_the_combined_default_file_manager_state() {
        let status = RegistrationStatus {
            context_menu: true,
            ..RegistrationStatus::default()
        };

        assert!(status.is_active());
        assert!(!status.is_complete());
    }
}
