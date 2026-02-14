use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobalSearchCategory {
    All,
    Files,
    Folders,
    Images,
    Videos,
    Audio,
    Documents,
}

pub struct GlobalSearchState {
    pub sender: Sender<crate::workers::global_search_worker::GlobalSearchRequest>,
    pub receiver: Receiver<crate::workers::global_search_worker::GlobalSearchResponse>,
    pub query: String,
    pub results: Vec<mtt_search_protocol::SearchResultItem>,
    pub selected_index: Option<usize>,
    pub focus_request: bool,
    pub size_cache: LruCache<String, Option<u64>>,
    pub category: GlobalSearchCategory,
    pub drive_filter: Option<char>,
    pub active: bool,
    pub loading: bool,
    pub requested_offset: u32,
    pub requested_limit: u32,
    pub has_more_results: bool,
    pub available: bool,
    pub last_check: Instant,
    pub total_indexed: u64,
}

impl GlobalSearchState {
    pub fn new(
        sender: Sender<crate::workers::global_search_worker::GlobalSearchRequest>,
        receiver: Receiver<crate::workers::global_search_worker::GlobalSearchResponse>,
    ) -> Self {
        Self {
            sender,
            receiver,
            query: String::new(),
            results: Vec::new(),
            selected_index: None,
            focus_request: false,
            size_cache: LruCache::new(
                NonZeroUsize::new(2000).expect("global_search size_cache size must be non-zero"),
            ),
            category: GlobalSearchCategory::All,
            drive_filter: None,
            active: false,
            loading: false,
            requested_offset: 0,
            requested_limit: 200,
            has_more_results: false,
            available: false,
            last_check: Instant::now(),
            total_indexed: 0,
        }
    }
}
