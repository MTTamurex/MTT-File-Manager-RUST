//! Synthetic index and real unique pipe; never opens the installed IPC endpoint.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{FromRawHandle, OwnedHandle};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use mtt_search_protocol::{decode_message, encode_message, SearchRequest, SearchResponse};
use windows::core::PCWSTR;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING,
};

use crate::file_index::{IndexState, VolumeIndex};
use crate::indexing_progress::IndexingProgress;
use crate::ipc_authorization::{inspect_file_manager_client, ClientIdentityError};
use crate::security_policy::IpcSecurityPolicy;
use crate::usn_journal::DiscoveredVolume;
use crate::volume_indices::{self, SharedVolumeIndices, VolumeIndexHandle};
use crate::volume_lifecycle::VolumeLifecycle;

use super::super::super::connection_lifecycle::ConnectionOutcome;
use super::super::super::handler::{self, WarmIndexState};
use super::super::create_pipe_named;
use super::fixture::{unique_name, Connection};
use super::lifecycle_worker::LifecycleWorker;
use super::restricted_token::{assert_no_thread_token, without_admin_rights};

pub(super) struct EndpointFixture {
    pub root: tempfile::TempDir,
    pub index: VolumeIndexHandle,
    pub hidden_frn: u64,
    indices: SharedVolumeIndices,
    progress: Arc<IndexingProgress>,
    lifecycle: Arc<VolumeLifecycle>,
}

impl EndpointFixture {
    pub fn new() -> Self {
        let root = tempfile::Builder::new()
            .prefix("mtt-endpoint-")
            .tempdir()
            .unwrap();
        std::fs::create_dir(root.path().join("restricted")).unwrap();
        std::fs::write(root.path().join("public.bin"), [0; 11]).unwrap();
        std::fs::write(root.path().join("blocked.bin"), [0; 37]).unwrap();
        std::fs::write(root.path().join("restricted").join("hidden.bin"), [0; 53]).unwrap();
        let root_path = root.path().to_str().unwrap();
        assert!(root_path.as_bytes()[0].is_ascii_alphabetic());
        assert_eq!(
            &root_path[1..3],
            ":\\",
            "fixture requires a drive-rooted temporary path"
        );
        let drive_letter = root_path.chars().next().unwrap().to_ascii_uppercase();
        let mut index = VolumeIndex::empty(drive_letter);
        index.volume_id = "synthetic-endpoint-volume".into();
        index.state = IndexState::Ready;
        index.sizes_loaded = true;

        // Only the fixture's directories need real identities for FolderSize's
        // live validation. Ancestors are names in memory, never scanned or edited.
        let root_frn = directory_frn(root.path());
        let restricted_frn = directory_frn(&root.path().join("restricted"));
        let first_file_frn = root_frn.max(restricted_frn).max(5) + 1;
        let hidden_frn = first_file_frn + 2;
        let mut parent = 5;
        for (next, name) in (first_file_frn + 3..).zip(root_path[3..].split('\\')) {
            assert!(index.insert_record(next, name, parent, true, false));
            parent = next;
        }
        let root_record = *index.records.get(&parent).unwrap();
        index.records.remove(&parent);
        index.records.insert(root_frn, root_record);
        assert!(index.insert_record(restricted_frn, "restricted", root_frn, true, false));
        for (frn, name, parent, size) in [
            (first_file_frn, "public.bin", root_frn, 11),
            (first_file_frn + 1, "blocked.bin", root_frn, 37),
            (hidden_frn, "hidden.bin", restricted_frn, 53),
        ] {
            assert!(index.insert_record(frn, name, parent, false, false));
            let record = index.records.get_mut(&frn).unwrap();
            record.size = size;
            record.allocated_size = size;
        }
        index.rebuild_children();
        index.dir_modified_at = HashMap::new();
        let volume = DiscoveredVolume {
            drive_letter,
            volume_id: index.volume_id.clone(),
            volume_serial: index.volume_serial,
            label: String::new(),
            file_system: "NTFS".into(),
            usn_supported: true,
        };
        let indices = volume_indices::new_shared();
        let index = volume_indices::upsert(&indices, index);
        let progress = Arc::new(IndexingProgress::new());
        let (wake, _receiver) = mpsc::channel();
        let lifecycle = VolumeLifecycle::new(
            Arc::new(AtomicBool::new(false)),
            Arc::clone(&indices),
            Arc::clone(&progress),
            wake,
        );
        // Registers synthetic runtime identity only; does not spawn an indexer.
        lifecycle.reconcile_discovered_volumes(&[volume]);
        Self {
            root,
            index,
            hidden_frn,
            indices,
            progress,
            lifecycle,
        }
    }

    pub fn path(&self, relative: &str) -> String {
        self.root.path().join(relative).to_str().unwrap().to_owned()
    }

    pub fn request(&self, request: SearchRequest) -> SearchResponse {
        self.request_inner(request, None)
    }

    // Only the identity result is simulated. Framing, impersonation, ACL checks,
    // index operations, response delivery and cleanup remain production code.
    pub fn request_with_identity(
        &self,
        request: SearchRequest,
        identity: Result<(), ClientIdentityError>,
    ) -> SearchResponse {
        self.request_inner(request, Some(identity))
    }

    pub fn request_with_unavailable_identity(&self, request: SearchRequest) -> SearchResponse {
        self.request_with_identity(
            request,
            Err(ClientIdentityError::Unavailable(
                "synthetic image-inspection failure".into(),
            )),
        )
    }

    fn request_inner(
        &self,
        request: SearchRequest,
        simulated_identity: Option<Result<(), ClientIdentityError>>,
    ) -> SearchResponse {
        let name = unique_name();
        assert_ne!(name, mtt_search_protocol::PIPE_NAME);
        let raw = create_pipe_named(&name, true).unwrap();
        let server = unsafe { OwnedHandle::from_raw_handle(raw.0) };
        let client = without_admin_rights(|| {
            OpenOptions::new()
                .read(true)
                .write(true)
                .access_mode(mtt_search_protocol::PIPE_CLIENT_ACCESS)
                // Static client context, impersonation allowed as in the app.
                .custom_flags(0x00100000 | 0x00020000)
                .open(&name)
                .expect("open unique endpoint test pipe")
        });
        let mut connection = Connection::from_parts(server, client);
        without_admin_rights(|| {
            connection
                .client
                .write_all(&encode_message(&request).unwrap())
                .unwrap();
        });
        let indices = Arc::clone(&self.indices);
        let progress = Arc::clone(&self.progress);
        let lifecycle = Arc::clone(&self.lifecycle);
        let task = LifecycleWorker::start(
            connection.clone_server(),
            Duration::from_secs(2),
            move |pipe, events| {
                let error = inspect_file_manager_client(pipe)
                    .expect_err("test client must not be trusted MTT");
                assert!(
                    matches!(error, ClientIdentityError::Rejected(_)),
                    "must reject a known foreign image, not fail to inspect its process"
                );
                let warming = Arc::new(AtomicBool::new(false));
                let epoch = Arc::new(AtomicU64::new(0));
                let warming = WarmIndexState {
                    is_warming: &warming,
                    last_warm_epoch_secs: &epoch,
                };
                let policy = IpcSecurityPolicy {
                    redact_status_metrics: false,
                };
                match simulated_identity {
                    None => handler::handle_client(
                        pipe, &indices, &progress, warming, &policy, &lifecycle, events,
                    ),
                    Some(identity) => handler::handle_client_with_verifier(
                        pipe,
                        &indices,
                        &progress,
                        warming,
                        &policy,
                        &lifecycle,
                        events,
                        |_| identity,
                    ),
                }
                assert_no_thread_token();
            },
        );
        let mut prefix = [0; 4];
        connection.client.read_exact(&mut prefix).unwrap();
        let length = u32::from_le_bytes(prefix) as usize;
        assert!((1..=65536).contains(&length));
        let mut payload = vec![0; length];
        connection.client.read_exact(&mut payload).unwrap();
        assert_eq!(task.wait(), ConnectionOutcome::Finished);
        decode_message(&payload).unwrap()
    }
}

fn directory_frn(path: &Path) -> u64 {
    let wide: Vec<u16> = path
        .to_str()
        .unwrap()
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            0x80, // FILE_READ_ATTRIBUTES; not directory listing.
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
    }
    .unwrap();
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    let result = unsafe { GetFileInformationByHandle(handle, &mut info) };
    unsafe {
        let _ = CloseHandle(handle);
    }
    result.unwrap();
    (((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64) & 0x0000_FFFF_FFFF_FFFF
}
