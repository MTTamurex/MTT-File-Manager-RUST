use crate::app::global_search_state::{
    CreatedMetadataState, GlobalSearchCategory, GlobalSearchTagFilter,
};
use rust_i18n::t;
use std::path::Path;

pub(super) fn category_label(category: GlobalSearchCategory) -> String {
    match category {
        GlobalSearchCategory::All => t!("search.category_all").to_string(),
        GlobalSearchCategory::Files => t!("search.category_files").to_string(),
        GlobalSearchCategory::Folders => t!("search.category_folders").to_string(),
        GlobalSearchCategory::Images => t!("search.category_images").to_string(),
        GlobalSearchCategory::Videos => t!("search.category_videos").to_string(),
        GlobalSearchCategory::Audio => t!("search.category_audio").to_string(),
        GlobalSearchCategory::Documents => t!("search.category_documents").to_string(),
    }
}

pub(crate) fn build_filtered_indices(
    results: &[mtt_search_protocol::SearchResultItem],
    category: GlobalSearchCategory,
    drive_filter: Option<char>,
    min_size_bytes: Option<u64>,
    max_size_bytes: Option<u64>,
    created_after: Option<u64>,
    created_before: Option<u64>,
    created_ts_cache: &[CreatedMetadataState],
    tag_filter: &GlobalSearchTagFilter,
    tag_assignments: &rustc_hash::FxHashMap<String, Vec<i64>>,
) -> Vec<usize> {
    let mut filtered = Vec::with_capacity(results.len());

    for (idx, result) in results.iter().enumerate() {
        // Drive filter
        if let Some(drive) = drive_filter {
            if extract_drive_letter(&result.full_path) != Some(drive) {
                continue;
            }
        }

        // Category filter
        if !matches_category(result, category) {
            continue;
        }

        // Size filter (directories always pass)
        if !result.is_dir {
            if let Some(min) = min_size_bytes {
                if result.size < min {
                    continue;
                }
            }
            if let Some(max) = max_size_bytes {
                if result.size > max {
                    continue;
                }
            }
        }

        // Created date filter: pending metadata remains visible until the request resolves.
        if created_after.is_some() || created_before.is_some() {
            if let Some(CreatedMetadataState::Available(cached_ts)) = created_ts_cache.get(idx) {
                if let Some(after) = created_after {
                    if *cached_ts < after {
                        continue;
                    }
                }
                if let Some(before) = created_before {
                    if *cached_ts > before {
                        continue;
                    }
                }
            } else if matches!(
                created_ts_cache.get(idx),
                Some(CreatedMetadataState::Unavailable)
            ) {
                continue;
            }
        }

        // Tag filter
        match tag_filter {
            GlobalSearchTagFilter::All => {
                // No tag filter applied.
            }
            GlobalSearchTagFilter::Any => {
                let path_tags = crate::domain::file_tag::tag_ids_for_path(
                    tag_assignments,
                    Path::new(&result.full_path),
                );
                let has_any = path_tags.is_some_and(|ids| !ids.is_empty());
                if !has_any {
                    continue;
                }
            }
            GlobalSearchTagFilter::Selected(required_ids) => {
                if required_ids.is_empty() {
                    // Defensive: empty Selected is equivalent to All.
                } else {
                    let path_tags = crate::domain::file_tag::tag_ids_for_path(
                        tag_assignments,
                        Path::new(&result.full_path),
                    );
                    let has_match = path_tags
                        .is_some_and(|ids| required_ids.iter().any(|tag_id| ids.contains(tag_id)));
                    if !has_match {
                        continue;
                    }
                }
            }
        }

        filtered.push(idx);
    }

    filtered
}

pub(crate) fn available_drives(results: &[mtt_search_protocol::SearchResultItem]) -> Vec<char> {
    let mut drives: Vec<char> = results
        .iter()
        .filter_map(|r| extract_drive_letter(&r.full_path))
        .collect();
    drives.sort_unstable();
    drives.dedup();
    drives
}

pub(super) fn extract_drive_letter(path: &str) -> Option<char> {
    use std::path::{Component, Path, Prefix};

    // Accept regular and verbatim Windows paths:
    // - C:\foo
    // - \\?\C:\foo
    // - \\.\C:\foo
    if let Some(Component::Prefix(prefix_component)) = Path::new(path).components().next() {
        match prefix_component.kind() {
            Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
                return Some((letter as char).to_ascii_uppercase());
            }
            _ => {}
        }
    }

    // Fallback for uncommon string forms (e.g., slashes normalized by other layers).
    let normalized = path
        .strip_prefix(r"\\?\")
        .or_else(|| path.strip_prefix(r"\\.\"))
        .or_else(|| path.strip_prefix("//?/"))
        .or_else(|| path.strip_prefix("//./"))
        .or_else(|| path.strip_prefix(r"\??\"))
        .unwrap_or(path);

    let mut chars = normalized.chars();
    let drive = chars.next()?.to_ascii_uppercase();
    if drive.is_ascii_alphabetic() && chars.next() == Some(':') {
        return Some(drive);
    }

    None
}

fn matches_category(
    result: &mtt_search_protocol::SearchResultItem,
    category: GlobalSearchCategory,
) -> bool {
    match category {
        GlobalSearchCategory::All => true,
        GlobalSearchCategory::Files => !result.is_dir,
        GlobalSearchCategory::Folders => result.is_dir,
        GlobalSearchCategory::Images => extension_in(
            &result.full_path,
            &[
                "jpg", "jpeg", "png", "gif", "bmp", "webp", "tiff", "tif", "svg", "heic", "avif",
                "ico",
            ],
        ),
        GlobalSearchCategory::Videos => extension_matches(
            &result.full_path,
            crate::infrastructure::windows::is_video_extension,
        ),
        GlobalSearchCategory::Audio => extension_in(
            &result.full_path,
            &["mp3", "wav", "flac", "aac", "ogg", "wma", "m4a", "opus"],
        ),
        GlobalSearchCategory::Documents => extension_in(
            &result.full_path,
            &[
                "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt", "md", "rtf", "odt",
                "csv",
            ],
        ),
    }
}

fn extension_in(path: &str, allowed: &[&str]) -> bool {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());

    let Some(ext) = ext else {
        return false;
    };

    allowed.iter().any(|candidate| *candidate == ext)
}

fn extension_matches(path: &str, predicate: fn(&str) -> bool) -> bool {
    std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(predicate)
}

pub(super) fn format_exact_number(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);

    for (idx, ch) in digits.chars().rev().enumerate() {
        if idx > 0 && idx % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }

    out.chars().rev().collect()
}

#[cfg(test)]
mod tests {
    use super::{build_filtered_indices, extract_drive_letter};
    use crate::app::global_search_state::{
        CreatedMetadataState, GlobalSearchCategory, GlobalSearchTagFilter,
    };
    use mtt_search_protocol::SearchResultItem;

    fn empty_cache() -> Vec<CreatedMetadataState> {
        Vec::new()
    }

    fn empty_assignments() -> rustc_hash::FxHashMap<String, Vec<i64>> {
        rustc_hash::FxHashMap::default()
    }

    #[test]
    fn extract_drive_letter_accepts_regular_windows_path() {
        assert_eq!(extract_drive_letter(r"C:\Users\foo.txt"), Some('C'));
        assert_eq!(extract_drive_letter(r"z:\vault\file.docx"), Some('Z'));
    }

    #[test]
    fn extract_drive_letter_accepts_verbatim_windows_path() {
        assert_eq!(extract_drive_letter(r"\\?\D:\data\file.bin"), Some('D'));
        assert_eq!(extract_drive_letter(r"\\.\E:\media\movie.mkv"), Some('E'));
    }

    #[test]
    fn extract_drive_letter_rejects_non_drive_paths() {
        assert_eq!(extract_drive_letter(r"\\server\share\file.txt"), None);
        assert_eq!(extract_drive_letter(r"/home/user/file.txt"), None);
    }

    #[test]
    fn video_filter_includes_ogm() {
        let results = vec![SearchResultItem {
            name: "sample.ogm".to_string(),
            full_path: r"C:\media\sample.ogm".to_string(),
            is_dir: false,
            size: 0,
        }];

        let filtered = build_filtered_indices(
            &results,
            GlobalSearchCategory::Videos,
            None,
            None,
            None,
            None,
            None,
            &empty_cache(),
            &GlobalSearchTagFilter::All,
            &empty_assignments(),
        );
        assert_eq!(filtered, vec![0]);
    }

    #[test]
    fn size_filter_min_and_max() {
        let results = vec![
            SearchResultItem {
                name: "small.txt".to_string(),
                full_path: r"C:\small.txt".to_string(),
                is_dir: false,
                size: 1024, // 1 KB
            },
            SearchResultItem {
                name: "medium.mp4".to_string(),
                full_path: r"C:\medium.mp4".to_string(),
                is_dir: false,
                size: 5 * 1024 * 1024, // 5 MB
            },
            SearchResultItem {
                name: "large.bin".to_string(),
                full_path: r"C:\large.bin".to_string(),
                is_dir: false,
                size: 100 * 1024 * 1024, // 100 MB
            },
            SearchResultItem {
                name: "dir".to_string(),
                full_path: r"C:\dir".to_string(),
                is_dir: true,
                size: 0,
            },
        ];

        let filtered = build_filtered_indices(
            &results,
            GlobalSearchCategory::All,
            None,
            Some(2 * 1024 * 1024),  // min 2 MB
            Some(50 * 1024 * 1024), // max 50 MB
            None,
            None,
            &empty_cache(),
            &GlobalSearchTagFilter::All,
            &empty_assignments(),
        );
        // small.txt excluded (below min), large.bin excluded (above max), dir included
        assert_eq!(filtered, vec![1, 3]);
    }

    #[test]
    fn size_filter_excludes_zero_size_below_min() {
        let results = vec![SearchResultItem {
            name: "empty.txt".to_string(),
            full_path: r"C:\empty.txt".to_string(),
            is_dir: false,
            size: 0,
        }];

        let filtered = build_filtered_indices(
            &results,
            GlobalSearchCategory::All,
            None,
            Some(1024 * 1024),
            None,
            None,
            None,
            &empty_cache(),
            &GlobalSearchTagFilter::All,
            &empty_assignments(),
        );
        assert!(filtered.is_empty());
    }

    #[test]
    fn created_date_filter_includes_pending_metadata() {
        let results = vec![SearchResultItem {
            name: "file.txt".to_string(),
            full_path: r"C:\file.txt".to_string(),
            is_dir: false,
            size: 100,
        }];
        let cache = vec![CreatedMetadataState::Pending];

        let filtered = build_filtered_indices(
            &results,
            GlobalSearchCategory::All,
            None,
            None,
            None,
            Some(1000), // created after ts=1000
            None,
            &cache,
            &GlobalSearchTagFilter::All,
            &empty_assignments(),
        );
        // Keep pending items visible while async metadata is loading.
        assert_eq!(filtered, vec![0]);
    }

    #[test]
    fn created_date_filter_excludes_unavailable_metadata() {
        let results = vec![SearchResultItem {
            name: "file.txt".to_string(),
            full_path: r"C:\file.txt".to_string(),
            is_dir: false,
            size: 100,
        }];
        let cache = vec![CreatedMetadataState::Unavailable];

        let filtered = build_filtered_indices(
            &results,
            GlobalSearchCategory::All,
            None,
            None,
            None,
            Some(1000),
            None,
            &cache,
            &GlobalSearchTagFilter::All,
            &empty_assignments(),
        );
        assert!(filtered.is_empty());
    }

    #[test]
    fn created_date_filter_excludes_out_of_range() {
        let results = vec![
            SearchResultItem {
                name: "old.txt".to_string(),
                full_path: r"C:\old.txt".to_string(),
                is_dir: false,
                size: 100,
            },
            SearchResultItem {
                name: "new.txt".to_string(),
                full_path: r"C:\new.txt".to_string(),
                is_dir: false,
                size: 200,
            },
        ];
        let cache = vec![
            CreatedMetadataState::Available(500),
            CreatedMetadataState::Available(2000),
        ];

        let filtered = build_filtered_indices(
            &results,
            GlobalSearchCategory::All,
            None,
            None,
            None,
            Some(1000), // created after ts=1000
            None,
            &cache,
            &GlobalSearchTagFilter::All,
            &empty_assignments(),
        );
        // old (500) excluded, new (2000) included
        assert_eq!(filtered, vec![1]);
    }

    // --- Tag filter tests ---

    fn sample_results() -> Vec<SearchResultItem> {
        vec![
            SearchResultItem {
                name: "tagged_a.txt".to_string(),
                full_path: r"C:\tagged_a.txt".to_string(),
                is_dir: false,
                size: 100,
            },
            SearchResultItem {
                name: "tagged_b.txt".to_string(),
                full_path: r"C:\tagged_b.txt".to_string(),
                is_dir: false,
                size: 100,
            },
            SearchResultItem {
                name: "tagged_both.txt".to_string(),
                full_path: r"C:\tagged_both.txt".to_string(),
                is_dir: false,
                size: 100,
            },
            SearchResultItem {
                name: "untagged.txt".to_string(),
                full_path: r"C:\untagged.txt".to_string(),
                is_dir: false,
                size: 100,
            },
        ]
    }

    fn assignments_for_sample() -> rustc_hash::FxHashMap<String, Vec<i64>> {
        let mut map = rustc_hash::FxHashMap::default();
        map.insert(r"c:\tagged_a.txt".to_string(), vec![1]);
        map.insert(r"c:\tagged_b.txt".to_string(), vec![2]);
        map.insert(r"c:\tagged_both.txt".to_string(), vec![1, 2]);
        // untagged.txt intentionally absent
        map
    }

    #[test]
    fn tag_filter_all_shows_tagged_and_untagged() {
        let filtered = build_filtered_indices(
            &sample_results(),
            GlobalSearchCategory::All,
            None,
            None,
            None,
            None,
            None,
            &empty_cache(),
            &GlobalSearchTagFilter::All,
            &assignments_for_sample(),
        );
        assert_eq!(filtered, vec![0, 1, 2, 3]);
    }

    #[test]
    fn tag_filter_any_excludes_untagged() {
        let filtered = build_filtered_indices(
            &sample_results(),
            GlobalSearchCategory::All,
            None,
            None,
            None,
            None,
            None,
            &empty_cache(),
            &GlobalSearchTagFilter::Any,
            &assignments_for_sample(),
        );
        // All results with at least one tag pass; untagged.txt excluded
        assert_eq!(filtered, vec![0, 1, 2]);
    }

    #[test]
    fn tag_filter_selected_or_semantics() {
        // Filter for tags 1 OR 2
        let filtered = build_filtered_indices(
            &sample_results(),
            GlobalSearchCategory::All,
            None,
            None,
            None,
            None,
            None,
            &empty_cache(),
            &GlobalSearchTagFilter::Selected(vec![1, 2]),
            &assignments_for_sample(),
        );
        // tagged_a (has 1) passes, tagged_b (has 2) passes, tagged_both passes, untagged fails
        assert_eq!(filtered, vec![0, 1, 2]);
    }

    #[test]
    fn tag_filter_selected_single_tag_narrows_results() {
        // Filter for tag 2 only
        let filtered = build_filtered_indices(
            &sample_results(),
            GlobalSearchCategory::All,
            None,
            None,
            None,
            None,
            None,
            &empty_cache(),
            &GlobalSearchTagFilter::Selected(vec![2]),
            &assignments_for_sample(),
        );
        // tagged_b and tagged_both pass; tagged_a and untagged fail
        assert_eq!(filtered, vec![1, 2]);
    }

    #[test]
    fn tag_filter_selected_no_match_returns_empty() {
        // Filter for tag 999 (not assigned to anything)
        let filtered = build_filtered_indices(
            &sample_results(),
            GlobalSearchCategory::All,
            None,
            None,
            None,
            None,
            None,
            &empty_cache(),
            &GlobalSearchTagFilter::Selected(vec![999]),
            &assignments_for_sample(),
        );
        assert!(filtered.is_empty());
    }
}
