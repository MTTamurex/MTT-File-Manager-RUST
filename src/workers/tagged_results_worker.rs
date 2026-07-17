use crate::app::global_search_state::GlobalSearchTagFilter;
use mtt_search_protocol::SearchResultItem;
use rustc_hash::FxHashMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;

pub type TaggedResultsCacheKey = (String, GlobalSearchTagFilter, u64);

pub struct TaggedResultsRequest {
    pub request_id: u64,
    pub cache_key: TaggedResultsCacheKey,
    pub tag_assignments: Arc<FxHashMap<PathBuf, Vec<i64>>>,
    pub excluded_paths: HashSet<String>,
    pub limit: usize,
}

pub struct TaggedResultsResponse {
    pub request_id: u64,
    pub cache_key: TaggedResultsCacheKey,
    pub items: Vec<SearchResultItem>,
    pub limit_reached: bool,
}

pub fn spawn_tagged_results_worker(
    ctx: eframe::egui::Context,
) -> std::io::Result<(
    Sender<TaggedResultsRequest>,
    Receiver<TaggedResultsResponse>,
    Arc<AtomicU64>,
)> {
    let (request_tx, request_rx) = mpsc::channel::<TaggedResultsRequest>();
    let (response_tx, response_rx) = mpsc::channel::<TaggedResultsResponse>();
    let active_request_id = Arc::new(AtomicU64::new(0));
    let worker_active_request_id = active_request_id.clone();

    std::thread::Builder::new()
        .name("tagged-results-worker".to_string())
        .spawn(move || {
            while let Ok(mut request) = request_rx.recv() {
                while let Ok(newer_request) = request_rx.try_recv() {
                    request = newer_request;
                }

                let Some(response) = build_tagged_results(request, &worker_active_request_id)
                else {
                    continue;
                };
                if response_tx.send(response).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        })?;

    Ok((request_tx, response_rx, active_request_id))
}

fn build_tagged_results(
    request: TaggedResultsRequest,
    active_request_id: &AtomicU64,
) -> Option<TaggedResultsResponse> {
    if active_request_id.load(Ordering::Relaxed) != request.request_id {
        return None;
    }

    let tokens: Vec<String> = request
        .cache_key
        .0
        .to_lowercase()
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    let mut seen_paths = request.excluded_paths;
    let mut items = Vec::new();

    for (path, tag_ids) in request.tag_assignments.iter() {
        if active_request_id.load(Ordering::Relaxed) != request.request_id {
            return None;
        }
        if items.len() >= request.limit {
            break;
        }
        if !tag_filter_matches_ids(&request.cache_key.1, tag_ids)
            || !path_name_matches_query(path, &tokens)
        {
            continue;
        }

        let path_text = path.to_string_lossy().into_owned();
        if !seen_paths.insert(normalize_search_path_key(&path_text)) {
            continue;
        }

        let Ok(metadata) = std::fs::metadata(path) else {
            continue;
        };
        if active_request_id.load(Ordering::Relaxed) != request.request_id {
            return None;
        }
        let Some(name) = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned)
        else {
            continue;
        };

        items.push(SearchResultItem {
            name,
            full_path: path_text,
            is_dir: metadata.is_dir(),
            size: if metadata.is_file() {
                metadata.len()
            } else {
                0
            },
        });
    }

    let limit_reached = items.len() >= request.limit;
    Some(TaggedResultsResponse {
        request_id: request.request_id,
        cache_key: request.cache_key,
        items,
        limit_reached,
    })
}

pub(crate) fn tag_filter_matches_ids(tag_filter: &GlobalSearchTagFilter, tag_ids: &[i64]) -> bool {
    match tag_filter {
        GlobalSearchTagFilter::All => true,
        GlobalSearchTagFilter::Any => !tag_ids.is_empty(),
        GlobalSearchTagFilter::Selected(required_ids) => required_ids
            .iter()
            .any(|required_id| tag_ids.contains(required_id)),
    }
}

pub(crate) fn path_name_matches_query(path: &Path, tokens: &[String]) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let name_lower = name.to_lowercase();
    tokens.iter().all(|token| name_lower.contains(token))
}

pub(crate) fn normalize_search_path_key(path: &str) -> String {
    let slash_normalized = path.replace('/', "\\");
    let stripped = slash_normalized
        .strip_prefix(r"\\?\")
        .or_else(|| slash_normalized.strip_prefix(r"\\.\"))
        .unwrap_or(&slash_normalized);

    if stripped.len() > 3 {
        stripped.trim_end_matches('\\').to_lowercase()
    } else {
        stripped.to_lowercase()
    }
}

#[cfg(test)]
mod tests {
    use super::{build_tagged_results, normalize_search_path_key, TaggedResultsRequest};
    use crate::app::global_search_state::GlobalSearchTagFilter;
    use rustc_hash::FxHashMap;
    use std::collections::HashSet;
    use std::sync::atomic::AtomicU64;
    use std::sync::Arc;

    #[test]
    fn builds_existing_tagged_results_and_skips_missing_paths() {
        let temp = tempfile::tempdir().unwrap();
        let matching = temp.path().join("Emma Notes.txt");
        std::fs::write(&matching, b"hello").unwrap();
        let missing = temp.path().join("Emma Missing.txt");
        let assignments = Arc::new(FxHashMap::from_iter([
            (matching.clone(), vec![5]),
            (missing, vec![5]),
        ]));
        let active = AtomicU64::new(1);

        let response = build_tagged_results(
            TaggedResultsRequest {
                request_id: 1,
                cache_key: (
                    "emma txt".to_string(),
                    GlobalSearchTagFilter::Selected(vec![5]),
                    0,
                ),
                tag_assignments: assignments,
                excluded_paths: HashSet::new(),
                limit: 10,
            },
            &active,
        )
        .unwrap();

        assert_eq!(response.items.len(), 1);
        assert_eq!(response.items[0].name, "Emma Notes.txt");
        assert_eq!(response.items[0].size, 5);
    }

    #[test]
    fn cancelled_request_produces_no_response() {
        let active = AtomicU64::new(2);
        let response = build_tagged_results(
            TaggedResultsRequest {
                request_id: 1,
                cache_key: ("file".to_string(), GlobalSearchTagFilter::Any, 0),
                tag_assignments: Arc::new(FxHashMap::default()),
                excluded_paths: HashSet::new(),
                limit: 10,
            },
            &active,
        );

        assert!(response.is_none());
    }

    #[test]
    fn normalizes_verbatim_case_and_separators() {
        assert_eq!(
            normalize_search_path_key(r"\\?\C:\Docs\Emma.txt"),
            normalize_search_path_key(r"c:/docs/emma.txt")
        );
    }
}
