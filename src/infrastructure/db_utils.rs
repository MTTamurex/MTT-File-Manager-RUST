//! Shared SQLite database utilities.
//!
//! Extracted from `disk_cache.rs` so both `ThumbnailDiskCache` and `AppStateDb`
//! can reuse the ACL hardening and connection setup logic.

use rusqlite::Connection;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

/// SEC: Get the raw SID bytes for the current process user from the process token.
/// Returns a buffer whose prefix is a valid SID structure.
pub fn get_current_user_sid_bytes() -> Option<(Vec<u8>, u32)> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::Security::{
        GetLengthSid, GetTokenInformation, IsValidSid, TokenUser, TOKEN_QUERY, TOKEN_USER,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token = windows::Win32::Foundation::HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).ok()?;

        let mut needed = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut needed);
        if needed == 0 {
            let _ = CloseHandle(token);
            return None;
        }

        let mut buf = vec![0u8; needed as usize];
        let ok = GetTokenInformation(
            token,
            TokenUser,
            Some(buf.as_mut_ptr() as *mut _),
            needed,
            &mut needed,
        );
        let _ = CloseHandle(token);
        ok.ok()?;

        let user_info = &*(buf.as_ptr() as *const TOKEN_USER);
        let sid = user_info.User.Sid;
        if !IsValidSid(sid).as_bool() {
            return None;
        }
        let sid_len = GetLengthSid(sid);
        let sid_ptr = sid.0 as *const u8;
        let sid_bytes = std::slice::from_raw_parts(sid_ptr, sid_len as usize).to_vec();
        Some((sid_bytes, sid_len))
    }
}

/// SEC: Apply an explicit DACL to a directory using Win32 API directly.
/// Grants the current user Full Control with inheritance, and removes inherited
/// permissions. This eliminates the TOCTOU window between directory creation and
/// ACL application.
pub fn harden_directory_permissions(dir: &Path) -> bool {
    use windows::Win32::Foundation::LocalFree;
    use windows::Win32::Security::Authorization::{
        SetEntriesInAclW, SetNamedSecurityInfoW, EXPLICIT_ACCESS_W, SE_FILE_OBJECT, SET_ACCESS,
        TRUSTEE_IS_SID, TRUSTEE_IS_USER, TRUSTEE_W,
    };
    use windows::Win32::Security::{
        ACE_FLAGS, ACL as WIN_ACL, DACL_SECURITY_INFORMATION,
        PROTECTED_DACL_SECURITY_INFORMATION,
    };

    let Some((mut user_sid_bytes, _sid_len)) = get_current_user_sid_bytes() else {
        log::warn!(
            "[DB-UTILS] Failed to get current user SID; skipping ACL hardening for {:?}",
            dir
        );
        return false;
    };

    // FILE_ALL_ACCESS = Full Control for the owner.
    const FILE_ALL_ACCESS: u32 = 0x001F01FF;

    // CONTAINER_INHERIT_ACE | OBJECT_INHERIT_ACE = sub-containers and objects inherit.
    let inheritance = ACE_FLAGS(3u32);

    let entries = [EXPLICIT_ACCESS_W {
        grfAccessPermissions: FILE_ALL_ACCESS,
        grfAccessMode: SET_ACCESS,
        grfInheritance: inheritance,
        Trustee: TRUSTEE_W {
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_USER,
            ptstrName: windows::core::PWSTR(user_sid_bytes.as_mut_ptr() as *mut u16),
            ..Default::default()
        },
    }];

    // Build the new ACL from the explicit entry.
    let mut new_acl = std::ptr::null_mut::<WIN_ACL>();
    let result = unsafe { SetEntriesInAclW(Some(&entries), None, &mut new_acl) };
    if result.0 != 0 {
        log::warn!(
            "[DB-UTILS] SetEntriesInAclW failed with code {} for {:?}",
            result.0,
            dir
        );
        return false;
    }

    // Apply the ACL to the directory. PROTECTED_DACL_SECURITY_INFORMATION removes
    // inherited ACEs (equivalent to `icacls /inheritance:r`).
    let dir_wide: Vec<u16> = OsStr::new(dir.as_os_str())
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let set_result = unsafe {
        SetNamedSecurityInfoW(
            windows::core::PCWSTR(dir_wide.as_ptr()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(new_acl as *const _),
            None,
        )
    };

    // Free the ACL allocated by SetEntriesInAclW.
    if !new_acl.is_null() {
        unsafe {
            LocalFree(Some(
                windows::Win32::Foundation::HLOCAL(new_acl as *mut _),
            ));
        }
    }

    if set_result.0 != 0 {
        log::warn!(
            "[DB-UTILS] SetNamedSecurityInfoW failed with code {} for {:?}",
            set_result.0,
            dir
        );
        return false;
    }

    true
}

/// Opens a temporary fallback database connection.
/// Tries to open on disk first (with ACL hardening); if that fails,
/// falls back to an in-memory database.
pub fn open_temp_fallback_connection(
    temp_fallback_path: &Path,
) -> rusqlite::Result<(Connection, Option<std::path::PathBuf>)> {
    if let Some(parent) = temp_fallback_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            log::warn!(
                "[DB-UTILS] Failed to ensure temporary fallback directory {:?}: {}",
                parent,
                e
            );
        }
    }

    let temp_parent_hardened = temp_fallback_path
        .parent()
        .map(|p| harden_directory_permissions(p))
        .unwrap_or(false);

    if !temp_parent_hardened {
        log::warn!(
            "[DB-UTILS] Temporary fallback directory ACL hardening failed. Using in-memory database instead."
        );
        return Ok((Connection::open_in_memory()?, None));
    }

    match Connection::open(temp_fallback_path) {
        Ok(c) => {
            log::warn!(
                "[DB-UTILS] Using temporary fallback database at {:?}",
                temp_fallback_path
            );
            Ok((c, Some(temp_fallback_path.to_path_buf())))
        }
        Err(temp_err) => {
            log::warn!(
                "[DB-UTILS] Failed to open temporary fallback database: {:?}. Using in-memory database.",
                temp_err
            );
            Ok((Connection::open_in_memory()?, None))
        }
    }
}

/// Applies WAL mode and NORMAL synchronous pragma to a connection.
pub fn apply_default_pragmas(conn: &Connection) {
    let _ = conn.execute("PRAGMA journal_mode = WAL", []).ok();
    let _ = conn.execute("PRAGMA synchronous = NORMAL", []).ok();
}
