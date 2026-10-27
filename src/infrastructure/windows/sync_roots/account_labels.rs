use super::{enum_subkeys, normalize_drive_root_for_compare, open_key, query_string};
use windows::Win32::System::Registry::HKEY_CURRENT_USER;

/// Read account labels during root discovery, never from the render loop.
pub(super) fn onedrive_account_display_name(root_path: &str) -> Option<String> {
    let accounts_path = "SOFTWARE\\Microsoft\\OneDrive\\Accounts";
    let accounts = open_key(HKEY_CURRENT_USER, accounts_path)?;
    for account in enum_subkeys(accounts.0) {
        let Some(key) = open_key(HKEY_CURRENT_USER, &format!("{accounts_path}\\{account}")) else {
            continue;
        };
        let Some(folder) = query_string(key.0, "UserFolder") else {
            continue;
        };
        if normalize_drive_root_for_compare(&folder) == normalize_drive_root_for_compare(root_path)
        {
            return query_string(key.0, "DisplayName").filter(|name| !name.trim().is_empty());
        }
    }
    None
}
