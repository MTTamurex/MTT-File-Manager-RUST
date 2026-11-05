//! Incremental policy: reject known foreign images, retain legacy path checks
//! only when identity inspection is unavailable. This is NOT strict MTT-only IPC.

use mtt_search_protocol::SearchResponse;
use windows::Win32::Foundation::HANDLE;

use crate::ipc_authorization::{
    current_client_can_read_path, trusted_file_manager_client, ClientIdentityError,
    PipeImpersonationGuard,
};

use super::pipe_io::send_response;

pub(super) fn require_index_client(
    pipe: HANDLE,
    operation: &str,
    folder_path: Option<&str>,
    verify: impl FnOnce(HANDLE) -> Result<(), ClientIdentityError>,
) -> bool {
    let authorization = authorize_request_with(verify(pipe), folder_path, |path| {
        let _guard = PipeImpersonationGuard::new(pipe)?;
        Ok(current_client_can_read_path(path))
    });
    match authorization {
        Ok(ClientAuthorization::TrustedClient) => true,
        Ok(ClientAuthorization::LegacyPathChecks { identity_error }) => {
            eprintln!(
                "[IPC] {operation} identity unavailable; retaining legacy path authorization: {}",
                crate::redact_paths(&identity_error),
            );
            true
        }
        Err(error) => {
            eprintln!(
                "[IPC] {operation} rejected unauthorized client: {}",
                crate::redact_paths(&error),
            );
            let _ = send_response(pipe, &SearchResponse::Error("Authorization failed".into()));
            false
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ClientAuthorization {
    TrustedClient,
    LegacyPathChecks { identity_error: String },
}

fn authorize_request_with(
    identity: Result<(), ClientIdentityError>,
    folder_path: Option<&str>,
    folder_acl: impl FnOnce(&str) -> Result<bool, String>,
) -> Result<ClientAuthorization, String> {
    match identity {
        Ok(()) => Ok(ClientAuthorization::TrustedClient),
        Err(ClientIdentityError::Rejected(reason)) => Err(reason),
        Err(ClientIdentityError::Unavailable(identity_error)) => {
            // Query and CheckPathsModified still perform their existing caller-
            // impersonated per-path checks inside the handler. Do not bypass them.
            if let Some(path) = folder_path {
                match folder_acl(path) {
                    Ok(true) => {}
                    Ok(false) => return Err(format!(
                        "client identity unavailable: {identity_error}; client cannot read requested path"
                    )),
                    Err(error) => return Err(format!(
                        "client identity unavailable: {identity_error}; ACL fallback failed: {error}"
                    )),
                }
            }
            Ok(ClientAuthorization::LegacyPathChecks { identity_error })
        }
    }
}

// These endpoints retain the original strict rule for unavailable identities too.
pub(super) fn require_trusted_metadata_client(pipe: HANDLE, operation: &str) -> bool {
    match trusted_file_manager_client(pipe) {
        Ok(()) => true,
        Err(error) => {
            eprintln!(
                "[IPC] {operation} rejected unauthorized client: {}",
                crate::redact_paths(&error),
            );
            let _ = send_response(pipe, &SearchResponse::Error("Authorization failed".into()));
            false
        }
    }
}

#[cfg(test)]
mod tests;
