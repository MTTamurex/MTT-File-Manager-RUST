use mtt_search_protocol::{SearchResultItem, VolumeStatus};

/// Requests sent from the UI to the global search worker.
pub enum GlobalSearchRequest {
    Search {
        query: String,
        offset: u32,
        limit: u32,
    },
    CheckStatus,
    SetStatusTracking {
        active: bool,
    },
}

/// Data results carry the indexing generation captured BEFORE querying the service.
pub enum GlobalSearchResponse {
    Results {
        indexing_generation: u64,
        query: String,
        items: Vec<SearchResultItem>,
        offset: u32,
        limit: u32,
        has_more: bool,
        total_matches: Option<u32>,
    },
    TotalCount {
        indexing_generation: u64,
        query: String,
        total_matches: u32,
    },
    Status {
        service_available: bool,
        available: bool,
        total_indexed: u64,
        session_total_indexed: u64,
        indexing_in_progress: bool,
        volumes: Vec<VolumeStatus>,
    },
    Error {
        indexing_generation: u64,
        query: String,
        message: String,
    },
}

impl GlobalSearchResponse {
    pub(crate) fn is_stale_for_indexing(&self, generation: u64) -> bool {
        match self {
            Self::Results {
                indexing_generation,
                ..
            }
            | Self::TotalCount {
                indexing_generation,
                ..
            }
            | Self::Error {
                indexing_generation,
                ..
            } => *indexing_generation != generation,
            Self::Status { .. } => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drive_indexing_generation_rejects_old_pages_counts_and_errors() {
        for response in [
            GlobalSearchResponse::Results {
                indexing_generation: 1,
                query: "file".into(),
                items: vec![],
                offset: 0,
                limit: 100,
                has_more: false,
                total_matches: Some(0),
            },
            GlobalSearchResponse::TotalCount {
                indexing_generation: 1,
                query: "file".into(),
                total_matches: 0,
            },
            GlobalSearchResponse::Error {
                indexing_generation: 1,
                query: "file".into(),
                message: "offline".into(),
            },
        ] {
            assert!(response.is_stale_for_indexing(2));
            assert!(!response.is_stale_for_indexing(1));
        }
    }
}
