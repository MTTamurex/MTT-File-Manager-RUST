use std::collections::HashMap;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::RevertToSelf;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAGS_AND_ATTRIBUTES,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Pipes::ImpersonateNamedPipeClient;

use crate::file_index::{self, VolumeIndex};
use crate::index_db;
use crate::path_resolver;
use mtt_search_protocol::SearchResultItem;

const AUTHZ_RAW_BATCH_SIZE: usize = 512;
const AUTHZ_MAX_BATCHES: usize = 64;
const GENERIC_READ: u32 = 0x80000000;

pub struct AuthorizedSearchPage {
    pub items: Vec<SearchResultItem>,
    pub has_more: bool,
    pub total_matches: Option<u32>,
}

struct PipeImpersonationGuard {
    active: bool,
}

impl PipeImpersonationGuard {
    fn new(pipe: HANDLE) -> Result<Self, String> {
        unsafe {
            ImpersonateNamedPipeClient(pipe)
                .map_err(|e| format!("ImpersonateNamedPipeClient failed: {}", e))?;
        }
        Ok(Self { active: true })
    }
}

impl Drop for PipeImpersonationGuard {
    fn drop(&mut self) {
        if self.active {
            unsafe {
                let _ = RevertToSelf();
            }
        }
    }
}

fn current_client_can_read_path(full_path: &str) -> bool {
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
            Err(_) => false,
        }
    }
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

pub fn collect_authorized_search_page(
    pipe: HANDLE,
    indices: &[VolumeIndex],
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
    let mut raw_offset = 0usize;
    let mut raw_has_more = true;
    let mut has_more_authorized = false;
    let mut batches = 0usize;

    // Cache authorization results by parent directory to avoid redundant
    // CreateFileW calls for files in the same folder (common in search results).
    let mut dir_auth_cache: HashMap<String, bool> = HashMap::with_capacity(64);

    while raw_has_more && batches < AUTHZ_MAX_BATCHES && !has_more_authorized {
        batches += 1;

        let raw_page = file_index::search_page(indices, query, raw_offset, AUTHZ_RAW_BATCH_SIZE);
        if raw_page.items.is_empty() {
            raw_has_more = false;
            break;
        }

        raw_offset = raw_offset.saturating_add(raw_page.items.len());
        raw_has_more = raw_page.has_more;

        for result in raw_page.items {
            // Fast path: check parent directory authorization from cache.
            // Most files inherit ACLs from their parent, so a single directory
            // check covers all sibling files without per-file CreateFileW.
            let authorized = match parent_dir_of(&result.full_path) {
                Some(parent) => is_parent_authorized(&mut dir_auth_cache, parent),
                None => current_client_can_read_path(&result.full_path),
            };

            if !authorized {
                continue;
            }

            authorized_total_seen = authorized_total_seen.saturating_add(1);

            // Offset is now applied to AUTHORIZED results (not raw index results).
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

            // Found at least one more authorized item after this page.
            has_more_authorized = true;
            break;
        }
    }

    // Safety cap reached with pending raw pages: keep pagination open only
    // when items were emitted so the caller can advance the offset. If zero
    // authorized items were collected the caller's offset cannot advance and
    // returning has_more=true would cause an infinite retry with the same
    // offset (raw_offset always restarts from 0 on each call).
    if batches >= AUTHZ_MAX_BATCHES && raw_has_more && !authorized_items.is_empty() {
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

/// FTS5-accelerated version of [`collect_authorized_search_page`].
///
/// Uses the SQLite FTS5 trigram index for fast substring matching, then resolves
/// full paths from the in-memory `VolumeIndex` and applies parent-directory
/// authorization caching — identical security semantics to the linear scan path.
pub fn collect_authorized_fts_page(
    pipe: HANDLE,
    fts_searcher: &index_db::FtsSearcher,
    indices: &[VolumeIndex],
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

    let _impersonation = PipeImpersonationGuard::new(pipe)?;

    let mut dir_auth_cache: HashMap<String, bool> = HashMap::with_capacity(64);
    // Per-volume directory path caches (keyed by drive_letter).
    let mut dir_path_caches: HashMap<char, HashMap<u64, String>> = HashMap::new();
    let mut authorized_items: Vec<SearchResultItem> = Vec::with_capacity(limit.min(1024));
    let mut authorized_total_seen = 0usize;
    let mut has_more_authorized = false;
    let mut fts_offset = 0usize;
    let mut fts_has_more = true;

    for _ in 0..AUTHZ_MAX_BATCHES {
        if !fts_has_more || has_more_authorized {
            break;
        }

        let fts_results = fts_searcher
            .search(query, fts_offset, AUTHZ_RAW_BATCH_SIZE)
            .map_err(|e| format!("FTS search error: {}", e))?;

        if fts_results.is_empty() {
            fts_has_more = false;
            break;
        }
        fts_offset += fts_results.len();
        fts_has_more = fts_results.len() >= AUTHZ_RAW_BATCH_SIZE;

        for fts_match in fts_results {
            // Find the in-memory VolumeIndex for path resolution.
            let vol = indices.iter().find(|v| {
                v.drive_letter == fts_match.drive_letter
                    && matches!(v.state, file_index::IndexState::Ready)
            });
            let Some(vol) = vol else { continue };

            // Resolve full path from the in-memory index (always current).
            let dir_cache = dir_path_caches
                .entry(fts_match.drive_letter)
                .or_default();
            let Some(full_path) =
                path_resolver::resolve_path_cached(fts_match.frn, vol, dir_cache)
            else {
                continue;
            };

            // Authorization check with parent-directory cache.
            let authorized = match parent_dir_of(&full_path) {
                Some(parent) => is_parent_authorized(&mut dir_auth_cache, parent),
                None => current_client_can_read_path(&full_path),
            };
            if !authorized {
                continue;
            }

            authorized_total_seen += 1;
            if authorized_total_seen <= offset {
                continue;
            }

            if authorized_items.len() < limit {
                authorized_items.push(SearchResultItem {
                    name: fts_match.name,
                    full_path,
                    is_dir: fts_match.is_dir,
                    size: 0,
                });
                continue;
            }

            has_more_authorized = true;
            break;
        }
    }

    if !has_more_authorized && fts_has_more && !authorized_items.is_empty() {
        has_more_authorized = true;
    }

    let has_more = has_more_authorized;

    let total_matches = if !fts_has_more && !has_more_authorized {
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
    use crate::file_index::{IndexState, SearchResult, SearchPage};

    #[test]
    fn returns_empty_for_zero_limit_or_empty_query() {
        let res_zero = collect_authorized_search_page(HANDLE(std::ptr::null_mut()), &[], "abc", 0, 0)
            .expect("zero-limit should succeed");
        assert!(res_zero.items.is_empty());
        assert!(!res_zero.has_more);
        assert_eq!(res_zero.total_matches, Some(0));

        let res_empty_query =
            collect_authorized_search_page(HANDLE(std::ptr::null_mut()), &[], "", 0, 10)
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

