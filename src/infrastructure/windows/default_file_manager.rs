//! Registers MTT as the per-user handler for opening folders and Win+E.
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
    pub win_e: bool,
}

impl RegistrationStatus {
    pub const fn is_active(self) -> bool {
        self.folder_open || self.folder_explore || self.win_e
    }

    pub const fn is_complete(self) -> bool {
        self.folder_open && self.folder_explore && self.win_e
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegistryActionError {
    Cancelled,
    Failed,
}

/// Read the three shell commands MTT manages for the current user.
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
        win_e: read_current_user_string(WIN_E_COMMAND_KEY, "")
            .is_some_and(|value| value.eq_ignore_ascii_case(&win_e_command)),
    }
}

/// Register MTT for opening folders and handling Win+E.
pub fn set_as_default(
    owner_window: Option<isize>,
) -> Result<RegistrationStatus, RegistryActionError> {
    apply_action(RegistryAction::Enable, owner_window)
}

/// Restore Windows Explorer only for values that still point to MTT.
pub fn restore_explorer(
    owner_window: Option<isize>,
) -> Result<RegistrationStatus, RegistryActionError> {
    apply_action(RegistryAction::RestoreExplorer, owner_window)
}

fn apply_action(
    action: RegistryAction,
    owner_window: Option<isize>,
) -> Result<RegistrationStatus, RegistryActionError> {
    let executable = std::env::current_exe().map_err(|_| RegistryActionError::Failed)?;
    let current_status = registration_status();

    if matches!(action, RegistryAction::RestoreExplorer) && !current_status.is_active() {
        return Ok(current_status);
    }

    let script = registry_script(action, &executable, current_status);
    import_registry_script(&script, owner_window)?;

    let status = registration_status();
    match action {
        RegistryAction::Enable if !status.is_complete() => {
            log::error!(
                "[DEFAULT-FILE-MANAGER] Registry import verification failed: folder_open={} folder_explore={} win_e={}",
                status.folder_open,
                status.folder_explore,
                status.win_e
            );
            Err(RegistryActionError::Failed)
        }
        RegistryAction::RestoreExplorer if status.is_active() => {
            log::error!(
                "[DEFAULT-FILE-MANAGER] Explorer restore verification failed: folder_open={} folder_explore={} win_e={}",
                status.folder_open,
                status.folder_explore,
                status.win_e
            );
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
            win_e: true,
        };

        assert!(status.is_active());
        assert!(!status.is_complete());
    }
}
