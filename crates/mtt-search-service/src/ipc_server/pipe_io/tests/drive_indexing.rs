//! Isolated transport and production dispatch; only client identity is injected.
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{mpsc, Arc};

use mtt_search_protocol::{
    decode_message, encode_message, SearchRequest, SearchResponse, DRIVE_INDEXING_DISABLED_ERROR,
};

use crate::index_db::IndexDb;
use crate::indexing_progress::IndexingProgress;
use crate::ipc_authorization::ClientIdentityError;
use crate::ipc_server::handler::{handle_client_with_verifier, WarmIndexState};
use crate::security_policy::IpcSecurityPolicy;
use crate::volume_indices;
use crate::volume_lifecycle::VolumeLifecycle;

use super::fixture::Connection;
use super::restricted_token::without_admin_rights;

struct Fixture {
    lifecycle: Arc<VolumeLifecycle>,
    _directory: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let db = Arc::new(IndexDb::open(&directory.path().join("index.db")).unwrap());
        let (wake, _) = mpsc::channel();
        let lifecycle = VolumeLifecycle::new(
            Arc::new(AtomicBool::new(false)),
            volume_indices::new_shared(),
            Arc::new(IndexingProgress::new()),
            wake,
        );
        lifecycle.initialize_drive_config(db).unwrap();
        Self {
            lifecycle,
            _directory: directory,
        }
    }

    fn request(
        &self,
        request: SearchRequest,
        identity: Result<(), ClientIdentityError>,
    ) -> SearchResponse {
        let mut connection = without_admin_rights(Connection::new);
        connection
            .client
            .write_all(&encode_message(&request).unwrap())
            .unwrap();
        let warming = Arc::new(AtomicBool::new(false));
        let epoch = Arc::new(AtomicU64::new(0));
        let (watchdog, _) = mpsc::channel();
        handle_client_with_verifier(
            connection.server(),
            &volume_indices::new_shared(),
            &Arc::new(IndexingProgress::new()),
            WarmIndexState {
                is_warming: &warming,
                last_warm_epoch_secs: &epoch,
            },
            &IpcSecurityPolicy {
                redact_status_metrics: false,
            },
            &self.lifecycle,
            &watchdog,
            |_| identity,
        );
        let mut prefix = [0; 4];
        connection.client.read_exact(&mut prefix).unwrap();
        let mut payload = vec![0; u32::from_le_bytes(prefix) as usize];
        connection.client.read_exact(&mut payload).unwrap();
        let response: SearchResponse = decode_message(&payload).unwrap();
        response.validate().unwrap();
        response
    }
}

#[test]
fn trusted_nonadministrator_can_write_and_read_machine_preferences() {
    let fixture = Fixture::new();
    assert!(matches!(
        fixture.request(
            SearchRequest::SetDriveIndexing {
                drive_letter: 'r',
                enabled: false
            },
            Ok(())
        ),
        SearchResponse::DriveIndexingUpdated {
            drive_letter: 'R',
            enabled: false
        }
    ));
    assert!(matches!(
        fixture.request(SearchRequest::GetDriveIndexingConfig, Ok(())),
        SearchResponse::DriveIndexingConfig { drives, .. }
            if drives.len() == 1 && drives[0].drive_letter == 'R' && !drives[0].enabled
    ));
}

#[test]
fn both_unavailable_and_rejected_identity_fail_closed_without_mutation() {
    let fixture = Fixture::new();
    for identity in [
        ClientIdentityError::Rejected("foreign image".into()),
        ClientIdentityError::Unavailable("identity unavailable".into()),
    ] {
        for request in [
            SearchRequest::GetDriveIndexingConfig,
            SearchRequest::SetDriveIndexing {
                drive_letter: 'R',
                enabled: false,
            },
        ] {
            assert!(matches!(
                fixture.request(request, Err(identity.clone())),
                SearchResponse::Error(error) if error == "Authorization failed"
            ));
            assert!(fixture.lifecycle.is_indexing_enabled('R'));
        }
    }
}

#[test]
fn invalid_drive_is_rejected_by_dispatch_validation() {
    let fixture = Fixture::new();
    assert!(matches!(
        fixture.request(SearchRequest::SetDriveIndexing { drive_letter: 'é', enabled: false }, Ok(())),
        SearchResponse::Error(error) if error == "Invalid request"
    ));
}

#[test]
fn disabled_folder_and_mixed_activity_requests_return_a_nonretryable_error() {
    let fixture = Fixture::new();
    fixture.lifecycle.set_drive_indexing('R', false).unwrap();
    for request in [
        SearchRequest::FolderSize {
            path: r"R:\folder".into(),
        },
        SearchRequest::CheckPathsModified {
            paths: vec![r"S:\folder".into(), r"R:\folder".into()],
            threshold_secs: 60,
        },
        SearchRequest::CheckPathsModified {
            paths: vec![r"\\?\r:\folder".into()],
            threshold_secs: 60,
        },
    ] {
        assert!(matches!(
            fixture.request(request, Ok(())),
            SearchResponse::Error(error) if error == DRIVE_INDEXING_DISABLED_ERROR
        ));
    }
}
