mod fts;
mod sync;

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use parking_lot::Mutex;

use rusqlite::{params, Connection};

pub use fts::FtsSearcher;

/// Persisted volume state for fast restart.
pub struct PersistedVolumeState {
    pub drive_letter: char,
    pub journal_id: u64,
    pub last_usn: i64,
    pub files_indexed: u64,
    pub has_hardlink_parent_data: bool,
    pub has_reparse_point_data: bool,
}

/// SQLite-based persistence for the file index.
/// Wrapped in Mutex because rusqlite::Connection is not Sync.
pub struct IndexDb {
    conn: Mutex<Connection>,
}

/// Get the database file path.
///
/// SEC: Hardcode `C:\ProgramData` instead of reading `%PROGRAMDATA%` env var.
/// A LocalSystem service always uses this path, and an attacker could redirect
/// the env var to an attacker-controlled directory to inject a malicious database.
pub fn get_db_path() -> Result<PathBuf, String> {
    let dir = Path::new(r"C:\ProgramData").join("MTT-File-Manager");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create ProgramData directory {:?}: {}", dir, e))?;

    // SEC: Apply ACLs via Win32 API directly, not via icacls subprocess.
    // Shelling out to icacls from a LocalSystem service is a DLL hijacking vector
    // and also has a TOCTOU window between directory creation and ACL application.
    harden_directory_acl(&dir)?;

    Ok(dir.join("search_index.db"))
}

/// Get a console-friendly database path that does not require ProgramData ACL
/// hardening. This is for local `run-console` diagnostics only; the Windows
/// service path remains fixed under `C:\ProgramData`.
pub fn get_console_db_path() -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join("MTT-File-Manager-Console");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create console DB directory {:?}: {}", dir, e))?;
    Ok(dir.join("search_index.db"))
}

/// Apply explicit DACL to the database directory using Win32 API.
/// Grants: SYSTEM (Full), Administrators (Full), Users (Read+Execute).
/// Removes inherited permissions.
fn harden_directory_acl(dir: &Path) -> Result<(), String> {
    use windows::Win32::Security::Authorization::{
        SetNamedSecurityInfoW, SE_FILE_OBJECT,
        SET_ACCESS,
        SetEntriesInAclW, EXPLICIT_ACCESS_W, TRUSTEE_W,
        TRUSTEE_IS_SID, TRUSTEE_IS_WELL_KNOWN_GROUP,
    };
    use windows::Win32::Security::{
        ACL as WIN_ACL, ACE_FLAGS,
        DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
    };
    use windows::Win32::Foundation::LocalFree;
    use windows::core::PCWSTR;

    // Build well-known SIDs inline (same pattern as pipe_io.rs).
    // SYSTEM: S-1-5-18 (revision=1, count=1, authority=5, sub=18)
    let mut sid_system = [0u8; 12];
    sid_system[0] = 1; // Revision
    sid_system[1] = 1; // SubAuthorityCount
    sid_system[7] = 5; // Identifier authority
    sid_system[8..12].copy_from_slice(&18u32.to_le_bytes()); // sub-authority: 18

    // Administrators: S-1-5-32-544 (revision=1, count=2, authority=5, sub=[32, 544])
    let mut sid_admins = [0u8; 16];
    sid_admins[0] = 1;
    sid_admins[1] = 2;
    sid_admins[7] = 5;
    sid_admins[8..12].copy_from_slice(&32u32.to_le_bytes());
    sid_admins[12..16].copy_from_slice(&544u32.to_le_bytes());

    // Users: S-1-5-32-545 (revision=1, count=2, authority=5, sub=[32, 545])
    let mut sid_users = [0u8; 16];
    sid_users[0] = 1;
    sid_users[1] = 2;
    sid_users[7] = 5;
    sid_users[8..12].copy_from_slice(&32u32.to_le_bytes());
    sid_users[12..16].copy_from_slice(&545u32.to_le_bytes());

    // FILE_ALL_ACCESS for SYSTEM and Administrators
    const FILE_ALL_ACCESS: u32 = 0x001F01FF;
    // FILE_GENERIC_READ | FILE_GENERIC_EXECUTE for Users
    const FILE_GENERIC_READ_EXECUTE: u32 = 0x001200A9;

    // CONTAINER_INHERIT_ACE | OBJECT_INHERIT_ACE = sub-containers and objects inherit
    let inheritance = ACE_FLAGS(3u32);

    let entries = [
        EXPLICIT_ACCESS_W {
            grfAccessPermissions: FILE_ALL_ACCESS,
            grfAccessMode: SET_ACCESS,
            grfInheritance: inheritance,
            Trustee: TRUSTEE_W {
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_WELL_KNOWN_GROUP,
                ptstrName: windows::core::PWSTR(sid_system.as_mut_ptr() as *mut u16),
                ..Default::default()
            },
        },
        EXPLICIT_ACCESS_W {
            grfAccessPermissions: FILE_ALL_ACCESS,
            grfAccessMode: SET_ACCESS,
            grfInheritance: inheritance,
            Trustee: TRUSTEE_W {
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_WELL_KNOWN_GROUP,
                ptstrName: windows::core::PWSTR(sid_admins.as_mut_ptr() as *mut u16),
                ..Default::default()
            },
        },
        EXPLICIT_ACCESS_W {
            grfAccessPermissions: FILE_GENERIC_READ_EXECUTE,
            grfAccessMode: SET_ACCESS,
            grfInheritance: inheritance,
            Trustee: TRUSTEE_W {
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_WELL_KNOWN_GROUP,
                ptstrName: windows::core::PWSTR(sid_users.as_mut_ptr() as *mut u16),
                ..Default::default()
            },
        },
    ];

    // Build the new ACL from the explicit entries.
    let mut new_acl = std::ptr::null_mut::<WIN_ACL>();
    let result = unsafe {
        SetEntriesInAclW(Some(&entries), None, &mut new_acl)
    };
    if result.0 != 0 {
        return Err(format!(
            "SetEntriesInAclW failed with error code {}",
            result.0
        ));
    }

    // Apply the ACL to the directory. PROTECTED_DACL_SECURITY_INFORMATION removes
    // inherited ACEs (equivalent to `icacls /inheritance:r`).
    let dir_wide: Vec<u16> = OsStr::new(dir.as_os_str())
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let set_result = unsafe {
        SetNamedSecurityInfoW(
            PCWSTR(dir_wide.as_ptr()),
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
        return Err(format!(
            "SetNamedSecurityInfoW failed with error code {}",
            set_result.0
        ));
    }

    Ok(())
}

impl IndexDb {
    pub fn open(path: &Path) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| format!("SQLite open error: {}", e))?;

        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA busy_timeout=10000;")
            .map_err(|e| format!("PRAGMA error: {}", e))?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS volume_state (
                drive_letter TEXT PRIMARY KEY,
                journal_id INTEGER NOT NULL,
                last_usn INTEGER NOT NULL,
                files_indexed INTEGER NOT NULL,
                last_full_scan_epoch INTEGER NOT NULL,
                has_hardlink_parent_data INTEGER NOT NULL DEFAULT 0,
                has_reparse_point_data INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS file_records (
                frn INTEGER NOT NULL,
                drive_letter TEXT NOT NULL,
                name TEXT NOT NULL,
                parent_frn INTEGER NOT NULL,
                is_dir INTEGER NOT NULL,
                is_reparse INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (drive_letter, frn)
            );
            CREATE TABLE IF NOT EXISTS hardlink_parents (
                drive_letter TEXT NOT NULL,
                frn INTEGER NOT NULL,
                parent_frn INTEGER NOT NULL,
                PRIMARY KEY (drive_letter, frn, parent_frn)
            );",
        )
        .map_err(|e| format!("Table creation error: {}", e))?;

        Self::migrate_schema(&conn)?;

        conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS search_fts USING fts5(
                name,
                content='file_records',
                content_rowid='rowid',
                tokenize='trigram'
            );",
        )
        .map_err(|e| format!("FTS5 table creation error: {}", e))?;

        let has_records: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM file_records LIMIT 1)",
                [],
                |row| row.get(0),
            )
            .unwrap_or(false);
        if has_records {
            let start = std::time::Instant::now();
            conn.execute(
                "INSERT INTO search_fts(search_fts) VALUES('rebuild')",
                [],
            )
            .map_err(|e| format!("FTS5 initial rebuild error: {}", e))?;
            eprintln!(
                "[DB] FTS5 index rebuilt at startup in {:.2}s",
                start.elapsed().as_secs_f64()
            );
        }

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn migrate_schema(conn: &Connection) -> Result<(), String> {
        let col_count: u32 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('file_records')",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if col_count == 7 {
            eprintln!("[DB] Migrating file_records from old 7-column schema to compact 5-column schema...");
            conn.execute_batch(
                "DROP TABLE IF EXISTS file_records;
                 CREATE TABLE file_records (
                     frn INTEGER NOT NULL,
                     drive_letter TEXT NOT NULL,
                     name TEXT NOT NULL,
                     parent_frn INTEGER NOT NULL,
                     is_dir INTEGER NOT NULL,
                     is_reparse INTEGER NOT NULL DEFAULT 0,
                     PRIMARY KEY (drive_letter, frn)
                 );",
            )
            .map_err(|e| format!("Schema migration error: {}", e))?;
            eprintln!("[DB] Migration complete. Index will be rebuilt on next scan.");
        }

        let has_hardlink_flag: u32 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('volume_state')
                 WHERE name = 'has_hardlink_parent_data'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        if has_hardlink_flag == 0 {
            eprintln!("[DB] Migrating volume_state to track hardlink parent completeness...");
            conn.execute(
                "ALTER TABLE volume_state
                 ADD COLUMN has_hardlink_parent_data INTEGER NOT NULL DEFAULT 0",
                [],
            )
            .map_err(|e| format!("Volume_state migration error: {}", e))?;
        }

        let has_reparse_flag: u32 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('volume_state')
                 WHERE name = 'has_reparse_point_data'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        if has_reparse_flag == 0 {
            eprintln!("[DB] Migrating volume_state to track reparse point completeness...");
            conn.execute(
                "ALTER TABLE volume_state
                 ADD COLUMN has_reparse_point_data INTEGER NOT NULL DEFAULT 0",
                [],
            )
            .map_err(|e| format!("Volume_state reparse migration error: {}", e))?;
        }

        let has_is_reparse_col: u32 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('file_records')
                 WHERE name = 'is_reparse'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        if has_is_reparse_col == 0 {
            eprintln!("[DB] Migrating file_records to track reparse points...");
            conn.execute(
                "ALTER TABLE file_records
                 ADD COLUMN is_reparse INTEGER NOT NULL DEFAULT 0",
                [],
            )
            .map_err(|e| format!("File_records reparse migration error: {}", e))?;
        }

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS hardlink_parents (
                drive_letter TEXT NOT NULL,
                frn INTEGER NOT NULL,
                parent_frn INTEGER NOT NULL,
                PRIMARY KEY (drive_letter, frn, parent_frn)
            );",
        )
        .map_err(|e| format!("Hardlink table creation error: {}", e))?;

        Ok(())
    }

    /// Load persisted volume state.
    pub fn load_volume_state(&self, drive_letter: char) -> Option<PersistedVolumeState> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT journal_id, last_usn, files_indexed, has_hardlink_parent_data, has_reparse_point_data
                 FROM volume_state WHERE drive_letter = ?1",
            )
            .ok()?;

        stmt.query_row(params![drive_letter.to_string()], |row| {
            Ok(PersistedVolumeState {
                drive_letter,
                journal_id: row.get::<_, i64>(0)? as u64,
                last_usn: row.get(1)?,
                files_indexed: row.get::<_, i64>(2)? as u64,
                has_hardlink_parent_data: row.get::<_, i64>(3)? != 0,
                has_reparse_point_data: row.get::<_, i64>(4)? != 0,
            })
        })
        .ok()
    }

    /// Stream file records from DB directly into the VolumeIndex's arena.
    /// Returns the number of records loaded, or None if no records found.
    pub fn load_into_index(
        &self,
        index: &mut crate::file_index::VolumeIndex,
    ) -> Option<usize> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT frn, name, parent_frn, is_dir, is_reparse
                 FROM file_records WHERE drive_letter = ?1",
            )
            .ok()?;

        let mut count = 0usize;
        let rows = stmt
            .query_map(params![index.drive_letter.to_string()], |row| {
                let frn: i64 = row.get(0)?;
                let name: String = row.get(1)?;
                let parent_frn: i64 = row.get(2)?;
                let is_dir: bool = row.get(3)?;
                let is_reparse: bool = row.get(4)?;
                Ok((frn as u64, name, parent_frn as u64, is_dir, is_reparse))
            })
            .ok()?;

        for (frn, name, parent_ref, is_dir, is_reparse) in rows.flatten() {
            if !index.insert_record(frn, &name, parent_ref, is_dir, is_reparse) {
                eprintln!("[INDEX-DB] Name arena full — stopping load for volume");
                break;
            }
            count += 1;
        }

        let mut hardlink_stmt = conn
            .prepare(
                "SELECT frn, parent_frn
                 FROM hardlink_parents WHERE drive_letter = ?1",
            )
            .ok()?;

        let hardlink_rows = hardlink_stmt
            .query_map(params![index.drive_letter.to_string()], |row| {
                let frn: i64 = row.get(0)?;
                let parent_frn: i64 = row.get(1)?;
                Ok((frn as u64, parent_frn as u64))
            })
            .ok()?;

        for (frn, parent_ref) in hardlink_rows.flatten() {
            let parents = index.hardlink_parents.entry(frn).or_default();
            if !parents.contains(&parent_ref) {
                parents.push(parent_ref);
            }
        }

        if count == 0 { None } else { Some(count) }
    }
}
