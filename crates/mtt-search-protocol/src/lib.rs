use serde::{Deserialize, Serialize};

/// Named pipe path for IPC between the search service and the file manager app.
pub const PIPE_NAME: &str = r"\\.\pipe\MTTFileManagerSearch";

/// Requests sent from the app to the search service.
#[derive(Serialize, Deserialize, Debug)]
pub enum SearchRequest {
    /// Search for files matching the query.
    Query { text: String, max_results: u32 },
    /// Get the current indexing status.
    GetStatus,
    /// Check if the service is alive.
    Ping,
    /// Ask the service to warm its in-memory index (bring paged-out memory back to RAM).
    WarmIndex,
}

/// Responses sent from the search service to the app.
#[derive(Serialize, Deserialize, Debug)]
pub enum SearchResponse {
    /// Search results.
    Results {
        items: Vec<SearchResultItem>,
        is_final: bool,
        total_found: u32,
    },
    /// Index status information.
    Status(IndexStatusInfo),
    /// Response to Ping.
    Pong,
    /// Acknowledge that index warming has started (or is already in progress).
    WarmStarted,
    /// Error message.
    Error(String),
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
    let payload = bincode::serialize(msg).map_err(|e| format!("serialization failed: {}", e))?;
    let len = (payload.len() as u32).to_le_bytes();
    let mut buf = Vec::with_capacity(4 + payload.len());
    buf.extend_from_slice(&len);
    buf.extend_from_slice(&payload);
    Ok(buf)
}

/// Read the length prefix from a buffer and decode the message.
pub fn decode_message<T: for<'de> Deserialize<'de>>(data: &[u8]) -> Result<T, String> {
    bincode::deserialize(data).map_err(|e| format!("deserialization failed: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundtrip_request() {
        let req = SearchRequest::Query {
            text: "test.txt".to_string(),
            max_results: 100,
        };
        let encoded = encode_message(&req).unwrap();
        // Skip 4-byte length prefix
        let decoded: SearchRequest = decode_message(&encoded[4..]).unwrap();
        match decoded {
            SearchRequest::Query { text, max_results } => {
                assert_eq!(text, "test.txt");
                assert_eq!(max_results, 100);
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
            is_final: true,
            total_found: 1,
        };
        let encoded = encode_message(&resp).unwrap();
        let decoded: SearchResponse = decode_message(&encoded[4..]).unwrap();
        match decoded {
            SearchResponse::Results {
                items, total_found, ..
            } => {
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].name, "hello.rs");
                assert_eq!(total_found, 1);
            }
            _ => panic!("unexpected variant"),
        }
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
