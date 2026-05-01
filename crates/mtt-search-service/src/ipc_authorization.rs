use std::collections::HashMap;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::RevertToSelf;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_FLAGS_AND_ATTRIBUTES, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Pipes::GetNamedPipeClientProcessId;
use windows::Win32::System::Pipes::ImpersonateNamedPipeClient;
use windows::Win32::System::Pipes::PeekNamedPipe;

use crate::file_index;
use crate::volume_indices::{self, SharedVolumeIndices};
use mtt_search_protocol::SearchResultItem;

const AUTHZ_RAW_BATCH_SIZE: usize = 512;
const AUTHZ_MAX_BATCHES: usize = 64;
const CLIENT_CONNECTION_CHECK_INTERVAL: usize = 16;
const GENERIC_READ: u32 = 0x80000000;

/// Maximum wall-clock time for the entire authorization loop.  Must be
/// shorter than the client-side pipe I/O timeout (8 s) so the service
/// always sends a response before the client gives up.  When the deadline
/// is reached, the service returns whatever authorized results it has
/// collected so far with `has_more: true`.
const AUTHZ_RESPONSE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(6);

pub struct AuthorizedSearchPage {
    pub items: Vec<SearchResultItem>,
    pub has_more: bool,
    pub total_matches: Option<u32>,
}

pub(crate) struct PipeImpersonationGuard {
    active: bool,
}

impl PipeImpersonationGuard {
    pub(crate) fn new(pipe: HANDLE) -> Result<Self, String> {
        match unsafe { ImpersonateNamedPipeClient(pipe) } {
            Ok(()) => Ok(Self { active: true }),
            Err(e) => Err(format!("ImpersonateNamedPipeClient failed: {}", e)),
        }
    }
}

impl Drop for PipeImpersonationGuard {
    fn drop(&mut self) {
        if self.active {
            if let Err(err) = unsafe { RevertToSelf() } {
                eprintln!(
                    "[IPC] RevertToSelf failed after pipe impersonation: {}",
                    err
                );
            }
            self.active = false;
        }
    }
}

pub(crate) fn current_client_can_read_path(full_path: &str) -> bool {
    let wide_path: Vec<u16> = OsStr::new(full_path)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let flags = FILE_FLAGS_AND_ATTRIBUTES(FILE_ATTRIBUTE_NORMAL.0 | FILE_FLAG_BACKUP_SEMANTICS.0);

    unsafe {
        match CreateFileW(
            PCWSTR(wide_path.as_ptr()),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            flags,
            None,
        ) {
            Ok(handle) => {
                let _ = CloseHandle(handle);
                true
            }
            Err(e) => {
                eprintln!(
                    "[IPC-AUTHZ] CreateFileW(GENERIC_READ) denied for {}: {}",
                    crate::redact_paths(full_path),
                    e
                );
                false
            }
        }
    }
}

pub(crate) fn trusted_file_manager_client(pipe: HANDLE) -> Result<(), String> {
    use windows::core::PWSTR;
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };

    let mut client_pid = 0u32;
    unsafe {
        GetNamedPipeClientProcessId(pipe, &mut client_pid)
            .map_err(|e| format!("GetNamedPipeClientProcessId failed: {}", e))?;
    }
    if client_pid == 0 {
        return Err("Client PID is 0".to_string());
    }

    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, client_pid) }
        .map_err(|e| format!("OpenProcess(client pid {}) failed: {}", client_pid, e))?;
    struct ProcGuard(HANDLE);
    impl Drop for ProcGuard {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
    let _guard = ProcGuard(process);

    let mut path_buf = [0u16; 4096];
    let mut path_len = path_buf.len() as u32;
    unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_FORMAT(0),
            PWSTR(path_buf.as_mut_ptr()),
            &mut path_len,
        )
        .map_err(|e| {
            format!(
                "QueryFullProcessImageNameW(client pid {}) failed: {}",
                client_pid, e
            )
        })?;
    }
    let image_path = String::from_utf16_lossy(&path_buf[..path_len as usize]);
    let basename = std::path::Path::new(&image_path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_ascii_lowercase())
        .unwrap_or_default();
    if basename != "mtt-file-manager.exe" {
        return Err(format!(
            "Client image basename is '{}', expected 'mtt-file-manager.exe'",
            basename
        ));
    }

    ensure_client_image_is_sibling(&image_path)
}

fn ensure_client_image_is_sibling(client_image_path: &str) -> Result<(), String> {
    let service_exe = std::env::current_exe().map_err(|e| format!("current_exe failed: {}", e))?;
    let service_dir = service_exe
        .parent()
        .ok_or_else(|| "Service executable has no parent directory".to_string())?;
    let client_dir = std::path::Path::new(client_image_path)
        .parent()
        .ok_or_else(|| "Client executable has no parent directory".to_string())?;

    if paths_equivalent_case_insensitive(service_dir, client_dir) {
        return Ok(());
    }

    Err(format!(
        "Client executable is not in the service directory: {}",
        crate::redact_paths(client_image_path)
    ))
}

fn paths_equivalent_case_insensitive(left: &std::path::Path, right: &std::path::Path) -> bool {
    let left_norm = std::fs::canonicalize(left).unwrap_or_else(|_| left.to_path_buf());
    let right_norm = std::fs::canonicalize(right).unwrap_or_else(|_| right.to_path_buf());
    left_norm
        .to_string_lossy()
        .eq_ignore_ascii_case(&right_norm.to_string_lossy())
}

/// Check if the impersonated client can read a parent directory.
/// Uses the same `CreateFileW` approach but with `FILE_FLAG_BACKUP_SEMANTICS`
/// which is required for opening directories.
#[inline]
fn parent_dir_of(full_path: &str) -> Option<&str> {
    // Fast rsearch for the last backslash (Windows paths).
    // Avoid allocating a PathBuf — just work with the raw string.
    let idx = full_path.rfind('\\')?;
    if idx == 0 {
        return None;
    }
    // Keep drive root like "C:\" intact (don't strip to "C:")
    if idx == 2 && full_path.as_bytes().get(1) == Some(&b':') {
        return Some(&full_path[..3]);
    }
    Some(&full_path[..idx])
}

/// Look up (or populate) the per-directory authorization cache.
/// Returns `true` if the impersonated client can read files in the given
/// directory, caching the result so subsequent files in the same directory
/// are answered without a syscall.
#[inline]
fn is_parent_authorized(cache: &mut HashMap<String, bool>, parent: &str) -> bool {
    if let Some(&cached) = cache.get(parent) {
        return cached;
    }
    let ok = current_client_can_read_path(parent);
    cache.insert(parent.to_owned(), ok);
    ok
}

#[inline]
fn ensure_client_connected(pipe: HANDLE) -> Result<(), String> {
    let mut total_avail = 0u32;
    unsafe {
        PeekNamedPipe(pipe, None, 0, None, Some(&mut total_avail), None)
            .map_err(|_| "Client disconnected".to_string())?;
    }
    Ok(())
}

pub fn collect_authorized_search_page(
    pipe: HANDLE,
    indices: &SharedVolumeIndices,
    query: &str,
    offset: usize,
    limit: usize,
) -> Result<AuthorizedSearchPage, String> {
    if limit == 0 || query.is_empty() {
        return Ok(AuthorizedSearchPage {
            items: Vec::new(),
            has_more: false,
            total_matches: Some(0),
        });
    }

    // All filesystem access checks below execute in the caller's security context.
    let _impersonation = PipeImpersonationGuard::new(pipe)?;

    let mut authorized_items: Vec<SearchResultItem> = Vec::with_capacity(limit.min(1024));
    let mut authorized_total_seen = 0usize;
    let mut has_more_authorized = false;

    // Cache authorization results by parent directory to avoid redundant
    // CreateFileW calls for files in the same folder (common in search results).
    let mut dir_auth_cache: HashMap<String, bool> = HashMap::with_capacity(64);
    let response_deadline = std::time::Instant::now() + AUTHZ_RESPONSE_DEADLINE;
    let mut deadline_exceeded = false;

    ensure_client_connected(pipe)?;
    let raw_scan_limit = AUTHZ_RAW_BATCH_SIZE.saturating_mul(AUTHZ_MAX_BATCHES);

    // Snapshot per-volume handles once and scan the raw index once for this
    // client request. The previous loop restarted `search_page` from record 0
    // for every raw batch and paid O(index * batches) under read locks before
    // authorization began.
    let raw_page = {
        let handles = volume_indices::snapshot_handles(indices);
        file_index::search_page(&handles, query, 0, raw_scan_limit)
    };
    let raw_has_more = raw_page.has_more;

    // Authorization runs WITHOUT the index lock held — CreateFileW, ACL
    // checks, and path resolution can now take arbitrarily long without
    // blocking writers (USN journal incremental updates).
    for (result_index, result) in raw_page.items.into_iter().enumerate() {
        if result_index.is_multiple_of(CLIENT_CONNECTION_CHECK_INTERVAL) {
            ensure_client_connected(pipe)?;
            if std::time::Instant::now() >= response_deadline {
                deadline_exceeded = true;
                break;
            }
        }

        let authorized = match parent_dir_of(&result.full_path) {
            Some(parent) => is_parent_authorized(&mut dir_auth_cache, parent),
            None => current_client_can_read_path(&result.full_path),
        };

        if !authorized {
            continue;
        }

        authorized_total_seen = authorized_total_seen.saturating_add(1);

        // Offset is applied to AUTHORIZED results, not raw index results.
        if authorized_total_seen <= offset {
            continue;
        }

        if authorized_items.len() < limit {
            authorized_items.push(SearchResultItem {
                name: result.name,
                full_path: result.full_path,
                is_dir: result.is_dir,
                size: 0,
            });
            continue;
        }

        has_more_authorized = true;
        break;
    }

    // Safety cap reached with pending raw pages: keep pagination open only
    // when items were emitted so the caller can advance the offset. If zero
    // authorized items were collected the caller's offset cannot advance and
    // returning has_more=true would cause an infinite retry with the same
    // offset (raw_offset always restarts from 0 on each call).
    if (raw_has_more || deadline_exceeded) && !authorized_items.is_empty() {
        has_more_authorized = true;
    }

    let has_more = has_more_authorized || (raw_has_more && !authorized_items.is_empty());

    let total_matches = if !raw_has_more && !has_more_authorized {
        Some(authorized_total_seen.min(u32::MAX as usize) as u32)
    } else {
        None
    };

    Ok(AuthorizedSearchPage {
        items: authorized_items,
        has_more,
        total_matches,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_index::{IndexState, SearchPage, SearchResult};

    #[test]
    fn returns_empty_for_zero_limit_or_empty_query() {
        let empty_indices = volume_indices::new_shared();
        let res_zero = collect_authorized_search_page(
            HANDLE(std::ptr::null_mut()),
            &empty_indices,
            "abc",
            0,
            0,
        )
        .expect("zero-limit should succeed");
        assert!(res_zero.items.is_empty());
        assert!(!res_zero.has_more);
        assert_eq!(res_zero.total_matches, Some(0));

        let res_empty_query =
            collect_authorized_search_page(HANDLE(std::ptr::null_mut()), &empty_indices, "", 0, 10)
                .expect("empty-query should succeed");
        assert!(res_empty_query.items.is_empty());
        assert!(!res_empty_query.has_more);
        assert_eq!(res_empty_query.total_matches, Some(0));
    }

    #[test]
    fn search_result_contract_still_matches_expected_shape() {
        let sample = SearchResult {
            name: "a.txt".to_string(),
            full_path: r"C:\temp\a.txt".to_string(),
            is_dir: false,
        };
        assert!(!sample.is_dir);
        assert!(sample.full_path.ends_with("a.txt"));

        let page = SearchPage {
            items: vec![sample],
            has_more: false,
            total_matches: Some(1),
        };
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.total_matches, Some(1));

        let state = IndexState::Ready;
        assert!(matches!(state, IndexState::Ready));
    }
}
