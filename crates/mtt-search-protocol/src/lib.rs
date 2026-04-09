use serde::{Deserialize, Serialize};

/// Named pipe path for IPC between the search service and the file manager app.
pub const PIPE_NAME: &str = r"\\.\pipe\MTTFileManagerSearch";

/// Maximum accepted query text length (bytes). Anything longer is likely
/// malformed or a deliberate abuse attempt, so we reject it early.
pub const MAX_QUERY_TEXT_LEN: usize = 1024;

/// Maximum result items we accept per response. Prevents a compromised or
/// buggy service from flooding the client with millions of entries.
pub const MAX_RESULT_ITEMS: usize = 10_000;

/// Maximum number of paths in a CheckPathsModified request.
pub const MAX_CHECK_PATHS: usize = 64;

/// Requests sent from the app to the search service.
#[derive(Serialize, Deserialize, Debug)]
pub enum SearchRequest {
    /// Search for files matching the query.
    Query {
        text: String,
        offset: u32,
        limit: u32,
    },
    /// Get the current indexing status.
    GetStatus,
    /// Check if the service is alive.
    Ping,
    /// Ask the service to warm its in-memory index (bring paged-out memory back to RAM).
    WarmIndex,
    /// Ask the service which of the given directory paths have been modified
    /// (via USN journal) within the last `threshold_secs` seconds.
    /// This allows the app to detect external changes without disk I/O.
    CheckPathsModified {
        paths: Vec<String>,
        threshold_secs: u32,
    },
}

impl SearchRequest {
    /// Validate deserialized fields to reject obviously malformed requests.
    pub fn validate(&self) -> Result<(), String> {
        if let SearchRequest::Query { text, limit, .. } = self {
            if text.len() > MAX_QUERY_TEXT_LEN {
                return Err(format!(
                    "query text too long ({} bytes, max {})",
                    text.len(),
                    MAX_QUERY_TEXT_LEN
                ));
            }
            if *limit > MAX_RESULT_ITEMS as u32 {
                return Err(format!(
                    "limit too large ({}, max {})",
                    limit, MAX_RESULT_ITEMS
                ));
            }
        }
        if let SearchRequest::CheckPathsModified { paths, .. } = self {
            if paths.len() > MAX_CHECK_PATHS {
                return Err(format!(
                    "too many paths ({}, max {})",
                    paths.len(),
                    MAX_CHECK_PATHS
                ));
            }
        }
        Ok(())
    }
}

/// Responses sent from the search service to the app.
#[derive(Serialize, Deserialize, Debug)]
pub enum SearchResponse {
    /// Search results.
    Results {
        items: Vec<SearchResultItem>,
        has_more: bool,
        total_matches: Option<u32>,
    },
    /// Index status information.
    Status(IndexStatusInfo),
    /// Response to Ping.
    Pong,
    /// Acknowledge that index warming has started (or is already in progress).
    WarmStarted,
    /// Directories from the request that have been modified within the threshold.
    PathsModified { modified: Vec<String> },
    /// Error message.
    Error(String),
}

impl SearchResponse {
    /// Validate deserialized response to reject pathologically large payloads.
    pub fn validate(&self) -> Result<(), String> {
        if let SearchResponse::Results { items, .. } = self {
            if items.len() > MAX_RESULT_ITEMS {
                return Err(format!(
                    "too many result items ({}, max {})",
                    items.len(),
                    MAX_RESULT_ITEMS
                ));
            }
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SearchResultItem {
    pub name: String,
    pub full_path: String,
    pub is_dir: bool,
    pub size: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct IndexStatusInfo {
    pub volumes: Vec<VolumeStatus>,
    pub total_files_indexed: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct VolumeStatus {
    pub drive_letter: char,
    /// "scanning", "ready", "error"
    pub state: String,
    pub files_indexed: u64,
}

/// Encode a message with a 4-byte little-endian length prefix for Named Pipe transport.
pub fn encode_message<T: Serialize>(msg: &T) -> Result<Vec<u8>, String> {
    use bincode::Options;
    let payload = bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .serialize(msg)
        .map_err(|e| format!("serialization failed: {}", e))?;
    let len = (payload.len() as u32).to_le_bytes();
    let mut buf = Vec::with_capacity(4 + payload.len());
    buf.extend_from_slice(&len);
    buf.extend_from_slice(&payload);
    Ok(buf)
}

/// Read the length prefix from a buffer and decode the message.
///
/// Uses an explicit byte limit to prevent malicious length-prefix inflation
/// (a small payload declaring multi-GB strings/vecs) from causing OOM panics.
/// The limit is set to the actual buffer size so bincode will reject any
/// internal length that exceeds it.
pub fn decode_message<T: for<'de> Deserialize<'de>>(data: &[u8]) -> Result<T, String> {
    use bincode::Options;
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(data.len() as u64)
        .deserialize(data)
        .map_err(|e| format!("deserialization failed: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundtrip_request() {
        let req = SearchRequest::Query {
            text: "test.txt".to_string(),
            offset: 50,
            limit: 100,
        };
        let encoded = encode_message(&req).unwrap();
        // Skip 4-byte length prefix
        let decoded: SearchRequest = decode_message(&encoded[4..]).unwrap();
        match decoded {
            SearchRequest::Query {
                text,
                offset,
                limit,
            } => {
                assert_eq!(text, "test.txt");
                assert_eq!(offset, 50);
                assert_eq!(limit, 100);
            }
            _ => panic!("unexpected variant"),
        }
    }

    #[test]
    fn test_roundtrip_response() {
        let resp = SearchResponse::Results {
            items: vec![SearchResultItem {
                name: "hello.rs".to_string(),
                full_path: r"C:\projects\hello.rs".to_string(),
                is_dir: false,
                size: 1024,
            }],
            has_more: true,
            total_matches: None,
        };
        let encoded = encode_message(&resp).unwrap();
        let decoded: SearchResponse = decode_message(&encoded[4..]).unwrap();
        let SearchResponse::Results {
            items,
            has_more,
            total_matches,
        } = decoded
        else {
            panic!("unexpected variant");
        };
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "hello.rs");
        assert!(has_more);
        assert!(total_matches.is_none());
    }

    #[test]
    fn test_ping_pong() {
        let req = SearchRequest::Ping;
        let encoded = encode_message(&req).unwrap();
        let decoded: SearchRequest = decode_message(&encoded[4..]).unwrap();
        assert!(matches!(decoded, SearchRequest::Ping));

        let resp = SearchResponse::Pong;
        let encoded = encode_message(&resp).unwrap();
        let decoded: SearchResponse = decode_message(&encoded[4..]).unwrap();
        assert!(matches!(decoded, SearchResponse::Pong));
    }
}
