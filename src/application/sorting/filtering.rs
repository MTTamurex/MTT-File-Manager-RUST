use crate::domain::file_entry::FileEntry;

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
pub(super) fn filter_items_opt(items: &[FileEntry], query: &str) -> Option<Vec<FileEntry>> {
    if query.is_empty() {
        return None;
    }

    let needle_lower: Vec<char> = query.chars().flat_map(|c| c.to_lowercase()).collect();
    let needle_ascii_lower: Option<Vec<u8>> = if needle_lower.iter().all(|c| c.is_ascii()) {
        Some(needle_lower.iter().map(|c| *c as u8).collect())
    } else {
        None
    };

    Some(
        items
            .iter()
            .filter(|item| {
                contains_ignore_case_precomputed(
                    &item.name,
                    &needle_lower,
                    needle_ascii_lower.as_deref(),
                )
            })
            .cloned()
            .collect(),
    )
}

/// Backwards-compatible filtering API.
pub(super) fn filter_items(items: &[FileEntry], query: &str) -> Vec<FileEntry> {
    filter_items_opt(items, query).unwrap_or_else(|| items.to_vec())
}
