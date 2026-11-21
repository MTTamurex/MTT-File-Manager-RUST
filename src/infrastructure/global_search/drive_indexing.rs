use super::{
    open_pipe, read_response, write_message, CONTROL_PIPE_IO_TIMEOUT_MS,
    DEFAULT_MAX_RESPONSE_PAYLOAD,
};
use crate::infrastructure::drive_indexing::{
    DriveIndexingError, DriveIndexingErrorKind, DriveIndexingSnapshot,
};
use mtt_search_protocol::{SearchRequest, SearchResponse};
use windows::Win32::Foundation::CloseHandle;

pub fn get_drive_indexing_config() -> Result<DriveIndexingSnapshot, DriveIndexingError> {
    config_response(exchange(&SearchRequest::GetDriveIndexingConfig)?)
}

pub fn set_drive_indexing(letter: char, enabled: bool) -> Result<(), DriveIndexingError> {
    if !letter.is_ascii_alphabetic() {
        return Err(DriveIndexingError::new(
            DriveIndexingErrorKind::InvalidDrive,
            "Drive letter must be an ASCII letter",
        ));
    }
    let letter = letter.to_ascii_uppercase();
    update_response(
        exchange(&SearchRequest::SetDriveIndexing {
            drive_letter: letter,
            enabled,
        })?,
        letter,
        enabled,
    )
}

fn exchange(request: &SearchRequest) -> Result<SearchResponse, DriveIndexingError> {
    let pipe = open_pipe().map_err(DriveIndexingError::transport)?;
    let result = (|| {
        write_message(pipe, request).map_err(DriveIndexingError::transport)?;
        read_response(
            pipe,
            CONTROL_PIPE_IO_TIMEOUT_MS,
            DEFAULT_MAX_RESPONSE_PAYLOAD,
        )
        .map_err(DriveIndexingError::transport)
    })();
    unsafe {
        let _ = CloseHandle(pipe);
    }
    result
}

fn config_response(response: SearchResponse) -> Result<DriveIndexingSnapshot, DriveIndexingError> {
    response.validate().map_err(DriveIndexingError::protocol)?;
    match response {
        SearchResponse::DriveIndexingConfig {
            drives,
            service_discovery_complete,
            enabled_service_volumes,
        } => {
            DriveIndexingSnapshot::new(drives, service_discovery_complete, enabled_service_volumes)
        }
        // This fieldless, valid request is rejected by pre-feature services.
        SearchResponse::Error(message) if message == "Invalid request" => {
            Err(DriveIndexingError::new(
                DriveIndexingErrorKind::Unsupported,
                "Drive indexing configuration is not supported",
            ))
        }
        SearchResponse::Error(message) => Err(DriveIndexingError::service(message)),
        _ => Err(DriveIndexingError::protocol(
            "Unexpected drive configuration response",
        )),
    }
}

fn update_response(
    response: SearchResponse,
    letter: char,
    enabled: bool,
) -> Result<(), DriveIndexingError> {
    response.validate().map_err(DriveIndexingError::protocol)?;
    match response {
        SearchResponse::DriveIndexingUpdated {
            drive_letter,
            enabled: actual,
        } if drive_letter == letter && actual == enabled => Ok(()),
        SearchResponse::Error(message) => Err(DriveIndexingError::service(message)),
        _ => Err(DriveIndexingError::protocol(
            "Drive indexing acknowledgement mismatch",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_explicit_legacy_rejection_means_unsupported() {
        for (message, expected) in [
            ("Invalid request", DriveIndexingErrorKind::Unsupported),
            ("Authorization failed", DriveIndexingErrorKind::AccessDenied),
            ("Server busy, try again later", DriveIndexingErrorKind::Busy),
            (
                "Drive configuration update failed",
                DriveIndexingErrorKind::Service,
            ),
        ] {
            assert_eq!(
                config_response(SearchResponse::Error(message.into()))
                    .unwrap_err()
                    .kind,
                expected
            );
        }
        assert_ne!(
            DriveIndexingError::transport("Search service timeout").kind,
            DriveIndexingErrorKind::Unsupported
        );
    }

    #[test]
    fn acknowledgement_must_match_both_letter_and_value() {
        for (letter, enabled) in [('S', false), ('R', true)] {
            let response = SearchResponse::DriveIndexingUpdated {
                drive_letter: letter,
                enabled,
            };
            assert!(update_response(response, 'R', false)
                .unwrap_err()
                .may_have_applied());
        }
        assert!(update_response(
            SearchResponse::DriveIndexingUpdated {
                drive_letter: 'R',
                enabled: false,
            },
            'R',
            false
        )
        .is_ok());
        assert_eq!(
            set_drive_indexing('é', false).unwrap_err().kind,
            DriveIndexingErrorKind::InvalidDrive
        );
    }
}
