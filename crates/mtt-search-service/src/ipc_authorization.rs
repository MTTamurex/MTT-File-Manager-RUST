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
            if !current_client_can_read_path(&result.full_path) {
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

    let total_matches = if !raw_has_more && !has_more_authorized {
        Some(authorized_total_seen.min(u32::MAX as usize) as u32)
    } else {
        None
    };

    Ok(AuthorizedSearchPage {
        items: authorized_items,
        has_more: has_more_authorized || raw_has_more,
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

