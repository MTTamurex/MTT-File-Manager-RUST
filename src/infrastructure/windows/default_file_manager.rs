//! Registers MTT as the per-user folder handler and context-menu command, with optional Win+E handling.
//!
//! Uses current-user registry APIs without elevation. Virtual Shell folders
//! retain their native handlers; Directory, Drive and filesystem Folder Shortcuts select the MTT verb.

mod network_location_open;
mod registration;
mod registry_access;
mod registry_transaction;

use registration::{
    class_is_active, folder_command, owns, win_e_command, RegistryAction, LEGACY_KEYS,
    WIN_E_COMMAND_KEY,
};
use registry_access::{read_current_user_string, UserRegistry};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RegistrationStatus {
    /// Directory default verb, or the legacy Folder open override.
    pub folder_open: bool,
    /// Drive default verb, or the legacy Folder explore override.
    pub folder_explore: bool,
    pub network_locations: bool,
    pub context_menu: bool,
    pub win_e: bool,
}

impl RegistrationStatus {
    /// Whether any part of the combined default-folder integration is active.
    pub const fn is_active(self) -> bool {
        self.folder_open || self.folder_explore || self.network_locations || self.context_menu
    }

    pub const fn has_folder_handlers(self) -> bool {
        self.folder_open || self.folder_explore || self.network_locations
    }

    /// Whether folder opening and the selected-folder verb are all registered.
    pub const fn is_complete(self) -> bool {
        self.folders_are_complete() && self.context_menu
    }

    pub const fn folders_are_complete(self) -> bool {
        self.folder_open && self.folder_explore && self.network_locations
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegistryActionError {
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
        network_locations: class_is_active(
            &UserRegistry,
            registration::NETWORK_LOCATION_CLASS,
            &registration::network_location_command(&executable),
        ),
        folder_open: class_is_active(&UserRegistry, "Directory", &folder_command)
            || owns(
                read_current_user_string(LEGACY_KEYS[0], "").as_deref(),
                &folder_command,
            ),
        folder_explore: class_is_active(&UserRegistry, "Drive", &folder_command)
            || owns(
                read_current_user_string(LEGACY_KEYS[1], "").as_deref(),
                &folder_command,
            ),
        context_menu:
            crate::infrastructure::windows::shell_context_menu::is_open_in_mtt_context_menu_enabled(
            ),
        win_e: read_current_user_string(WIN_E_COMMAND_KEY, "")
            .is_some_and(|value| value.eq_ignore_ascii_case(&win_e_command)),
    }
}

/// Upgrade owned legacy or pre-network-location registrations before Shell launch requests.
pub fn migrate_legacy_registration() -> Result<(), RegistryActionError> {
    let executable = std::env::current_exe().map_err(|_| RegistryActionError::Failed)?;
    let command = folder_command(&executable);
    if registration::needs_network_location_upgrade(&UserRegistry, &executable)
        || LEGACY_KEYS
            .iter()
            .any(|key| owns(read_current_user_string(key, "").as_deref(), &command))
    {
        apply_action(RegistryAction::EnableFolders)?;
    }
    Ok(())
}

/// Shell namespace identifiers must never be sent to the MTT folder UI.
pub fn is_supported_folder_target(path: &std::path::Path) -> bool {
    if crate::infrastructure::security::classify_shell_namespace_path(path).is_some() {
        return false;
    }
    super::network_locations::is_network_target(path)
        || super::network_locations::resolve_network_location_target(path).is_some()
        || crate::infrastructure::onedrive::fast_is_dir(path)
}

/// Preserve native opening for unsupported Folder Shortcut protocols (for example FTP).
pub fn open_network_location_in_explorer(path: &std::path::Path) -> std::io::Result<()> {
    network_location_open::open_in_explorer(path)
}

pub fn resolve_network_location_open_target(path: &std::path::Path) -> Option<std::path::PathBuf> {
    if super::network_locations::is_network_target(path) {
        Some(path.to_path_buf())
    } else {
        super::network_locations::resolve_folder_shortcut_network_target(path)
    }
}

/// Register MTT for folder opening and add its folder context-menu command, without changing Win+E.
pub fn set_as_default(context_menu_label: &str) -> Result<RegistrationStatus, RegistryActionError> {
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
    if let Err(error) = apply_action(RegistryAction::EnableFolders) {
        if !context_menu_was_enabled {
            let _ = crate::infrastructure::windows::shell_context_menu::
                set_open_in_mtt_context_menu_enabled(false, context_menu_label);
        }
        return Err(error);
    }

    let status = registration_status();
    if !status.is_complete() {
        log::error!(
            "[DEFAULT-FILE-MANAGER] Combined registration verification failed: folder_open={} folder_explore={} network_locations={} context_menu={}",
            status.folder_open,
            status.folder_explore,
            status.network_locations,
            status.context_menu
        );
        return Err(RegistryActionError::Failed);
    }
    Ok(status)
}

/// Restore Windows Explorer's folder-open handlers and remove Open in MTT, without changing Win+E.
pub fn restore_explorer(
    context_menu_label: &str,
) -> Result<RegistrationStatus, RegistryActionError> {
    apply_action(RegistryAction::RestoreFolders)?;
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
pub fn set_win_e_enabled(enabled: bool) -> Result<RegistrationStatus, RegistryActionError> {
    let action = if enabled {
        RegistryAction::EnableWinE
    } else {
        RegistryAction::RestoreWinE
    };
    apply_action(action)
}

fn apply_action(action: RegistryAction) -> Result<RegistrationStatus, RegistryActionError> {
    let executable = std::env::current_exe().map_err(|_| RegistryActionError::Failed)?;
    let current_status = registration_status();

    if matches!(action, RegistryAction::RestoreWinE) && !current_status.win_e {
        return Ok(current_status);
    }

    let result = registration::apply(&mut UserRegistry, action, &executable);
    // Notify even after a partial failure so the Shell refreshes any completed writes.
    unsafe {
        use windows::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};
        SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None);
    }
    result.map_err(|error| {
        log::error!("[DEFAULT-FILE-MANAGER] {error}");
        RegistryActionError::Failed
    })?;

    let status = registration_status();
    match action {
        RegistryAction::EnableFolders if !status.folders_are_complete() => {
            log::error!(
                "[DEFAULT-FILE-MANAGER] Registry verification failed: folder_open={} folder_explore={} network_locations={}",
                status.folder_open,
                status.folder_explore,
                status.network_locations
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
            log::error!("[WIN-E-INTEGRATION] Registry verification failed");
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
    fn complete_registration_includes_saved_network_location_opening() {
        let mut status = RegistrationStatus {
            folder_open: true,
            folder_explore: true,
            context_menu: true,
            ..Default::default()
        };
        assert!(!status.is_complete());
        status.network_locations = true;
        assert!(status.is_complete());
    }

    #[test]
    fn registration_status_distinguishes_partial_registration() {
        let status = RegistrationStatus {
            folder_open: true,
            folder_explore: false,
            context_menu: false,
            win_e: true,
            network_locations: false,
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
            network_locations: false,
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
