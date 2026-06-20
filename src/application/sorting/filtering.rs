use crate::domain::file_entry::FileEntry;
use crate::domain::file_tag;
use rustc_hash::FxHashMap;
use std::path::PathBuf;

/// Check if haystack contains needle (case-insensitive) using precomputed needle.
/// Fast path for ASCII, fallback to Unicode-aware comparison.
#[inline]
fn contains_ignore_case_precomputed(
    haystack: &str,
    needle_lower: &[char],
    needle_ascii_lower: Option<&[u8]>,
) -> bool {
    if needle_lower.is_empty() {
        return true;
    }

    // Fast path for ASCII strings (majority of filenames).
    if haystack.is_ascii() {
        if let Some(needle_bytes) = needle_ascii_lower {
            return haystack.as_bytes().windows(needle_bytes.len()).any(|w| {
                w.iter()
                    .zip(needle_bytes.iter())
                    .all(|(h, n)| h.to_ascii_lowercase() == *n)
            });
        }
    }

    // Fallback: Unicode-aware comparison.
    let haystack_chars: Vec<char> = haystack.chars().flat_map(|c| c.to_lowercase()).collect();
    haystack_chars
        .windows(needle_lower.len())
        .any(|window| window == needle_lower)
}

/// Filters items based on a query string.
///
/// When query is empty, returns None to signal "use all items" without cloning.
///
/// Multi-word queries use token-based matching (same strategy as global search):
/// the query is split by whitespace and every token must appear as a
/// case-insensitive substring of the filename.
pub(super) fn filter_items_opt(items: &[FileEntry], query: &str) -> Option<Vec<FileEntry>> {
    if query.is_empty() {
        return None;
    }

    // Split query into tokens (same strategy as global search).
    let tokens: Vec<&str> = query.split_whitespace().collect();
    if tokens.is_empty() {
        return None;
    }

    // Precompute lowered representations for each token.
    let precomputed: Vec<(Vec<char>, Option<Vec<u8>>)> = tokens
        .iter()
        .map(|token| {
            let lower: Vec<char> = token.chars().flat_map(|c| c.to_lowercase()).collect();
            let ascii = if lower.iter().all(|c| c.is_ascii()) {
                Some(lower.iter().map(|c| *c as u8).collect())
            } else {
                None
            };
            (lower, ascii)
        })
        .collect();

    Some(
        items
            .iter()
            .filter(|item| {
                precomputed.iter().all(|(needle_lower, needle_ascii)| {
                    contains_ignore_case_precomputed(
                        &item.name,
                        needle_lower,
                        needle_ascii.as_deref(),
                    )
                })
            })
            .cloned()
            .collect(),
    )
}

/// Filters items by name query and optional tag assignment.
///
/// Returns None when both filters are inactive so callers can keep the existing
/// no-clone fast path.
pub(super) fn filter_items_opt_with_tags(
    items: &[FileEntry],
    query: &str,
    active_tag_filter: Option<i64>,
    tag_assignments: &FxHashMap<PathBuf, Vec<i64>>,
) -> Option<Vec<FileEntry>> {
    let has_tag_filter = active_tag_filter.is_some();
    if query.is_empty() && !has_tag_filter {
        return None;
    }

    let tokens: Vec<&str> = query.split_whitespace().collect();
    let has_query = !tokens.is_empty();
    if !has_query && !has_tag_filter {
        return None;
    }

    let precomputed: Vec<(Vec<char>, Option<Vec<u8>>)> = if has_query {
        tokens
            .iter()
            .map(|token| {
                let lower: Vec<char> = token.chars().flat_map(|c| c.to_lowercase()).collect();
                let ascii = if lower.iter().all(|c| c.is_ascii()) {
                    Some(lower.iter().map(|c| *c as u8).collect())
                } else {
                    None
                };
                (lower, ascii)
            })
            .collect()
    } else {
        Vec::new()
    };

    Some(
        items
            .iter()
            .filter(|item| {
                let name_matches = !has_query
                    || precomputed.iter().all(|(needle_lower, needle_ascii)| {
                        contains_ignore_case_precomputed(
                            &item.name,
                            needle_lower,
                            needle_ascii.as_deref(),
                        )
                    });
                let tag_matches = active_tag_filter.map_or(true, |tag_id| {
                    file_tag::path_has_tag(tag_assignments, &item.path, tag_id)
                });
                name_matches && tag_matches
            })
            .cloned()
            .collect(),
    )
}

/// Backwards-compatible filtering API.
pub(super) fn filter_items(items: &[FileEntry], query: &str) -> Vec<FileEntry> {
    filter_items_opt(items, query).unwrap_or_else(|| items.to_vec())
}
