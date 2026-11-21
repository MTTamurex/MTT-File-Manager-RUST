use serde::{Deserialize, Serialize};

/// Stable wire error; clients must not retry it as an index-startup failure.
pub const DRIVE_INDEXING_DISABLED_ERROR: &str = "Drive indexing disabled";

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct DriveIndexingEntry {
    pub drive_letter: char,
    pub enabled: bool,
}

pub(crate) fn validate_drive_letters(
    letters: impl IntoIterator<Item = char>,
) -> Result<(), String> {
    let mut seen = 0u32;
    for letter in letters {
        if !letter.is_ascii_uppercase() {
            return Err("drive letter must be uppercase ASCII".into());
        }
        let bit = 1u32 << (letter as u32 - 'A' as u32);
        if seen & bit != 0 {
            return Err("duplicate drive letter".into());
        }
        seen |= bit;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{decode_message, encode_message, SearchRequest, SearchResponse};

    #[test]
    fn configuration_requests_roundtrip_and_accept_lowercase_input() {
        for request in [
            SearchRequest::GetDriveIndexingConfig,
            SearchRequest::SetDriveIndexing {
                drive_letter: 'r',
                enabled: false,
            },
        ] {
            let bytes = encode_message(&request).unwrap();
            let decoded: SearchRequest = decode_message(&bytes[4..]).unwrap();
            assert!(decoded.validate().is_ok());
            assert_eq!(bytes, encode_message(&decoded).unwrap());
        }
        for drive_letter in ['1', ':', 'é', '\0'] {
            assert!(SearchRequest::SetDriveIndexing {
                drive_letter,
                enabled: false
            }
            .validate()
            .is_err());
        }
    }

    #[test]
    fn configuration_responses_roundtrip() {
        for response in [
            SearchResponse::DriveIndexingConfig {
                drives: vec![DriveIndexingEntry {
                    drive_letter: 'R',
                    enabled: false,
                }],
                service_discovery_complete: true,
                enabled_service_volumes: vec![],
            },
            SearchResponse::DriveIndexingUpdated {
                drive_letter: 'R',
                enabled: true,
            },
        ] {
            let bytes = encode_message(&response).unwrap();
            let decoded: SearchResponse = decode_message(&bytes[4..]).unwrap();
            assert!(decoded.validate().is_ok());
            assert_eq!(bytes, encode_message(&decoded).unwrap());
        }
    }

    #[test]
    fn configuration_rejects_duplicate_invalid_or_inconsistent_letters() {
        for letters in [vec!['R', 'R'], vec!['r'], vec!['é']] {
            assert!(validate_drive_letters(letters).is_err());
        }
        assert!(SearchResponse::DriveIndexingConfig {
            drives: vec![DriveIndexingEntry {
                drive_letter: 'R',
                enabled: false
            }],
            service_discovery_complete: true,
            enabled_service_volumes: vec!['R'],
        }
        .validate()
        .is_err());
    }

    #[test]
    fn existing_wire_discriminants_remain_unchanged() {
        // Fixed-width bincode enum tags are little-endian u32 values.
        for (request, tag) in [
            (SearchRequest::GetStatus, 1u32),
            (SearchRequest::Ping, 2),
            (SearchRequest::WarmIndex, 3),
            (SearchRequest::DiskAnalysis { drive_letter: 'R' }, 7),
            (SearchRequest::GetDriveIndexingConfig, 8),
            (
                SearchRequest::SetDriveIndexing {
                    drive_letter: 'R',
                    enabled: false,
                },
                9,
            ),
        ] {
            assert_eq!(&encode_message(&request).unwrap()[4..8], &tag.to_le_bytes());
        }
        assert_eq!(
            encode_message(&SearchRequest::Ping).unwrap(),
            vec![4, 0, 0, 0, 2, 0, 0, 0]
        );
        assert_eq!(
            encode_message(&SearchResponse::Pong).unwrap(),
            vec![4, 0, 0, 0, 2, 0, 0, 0]
        );
        for (response, tag) in [
            (SearchResponse::Error("Invalid request".into()), 6u32),
            (
                SearchResponse::DriveIndexingUpdated {
                    drive_letter: 'R',
                    enabled: false,
                },
                10,
            ),
        ] {
            assert_eq!(
                &encode_message(&response).unwrap()[4..8],
                &tag.to_le_bytes()
            );
        }
    }
}
