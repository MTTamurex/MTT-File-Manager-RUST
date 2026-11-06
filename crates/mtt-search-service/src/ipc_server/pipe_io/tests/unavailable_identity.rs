//! Snapshot rejection with actual image-query failures and readable fixture paths.

use mtt_search_protocol::{SearchRequest, SearchResponse};

use crate::file_index::FileMetrics;
use crate::ipc_authorization::{current_client_can_read_path, with_restricting_sid};

use super::endpoint_acl::RestrictedRead;
use super::endpoint_fixture::EndpointFixture;

#[test]
fn snapshot_rejects_foreign_client_despite_readable_folder() {
    let fixture = EndpointFixture::new();
    with_restricting_sid(|| assert!(!current_client_can_read_path(&fixture.path(""))));
    let access = RestrictedRead::root(&fixture.root);
    with_restricting_sid(|| {
        assert!(current_client_can_read_path(&fixture.path("")));
        assert!(!current_client_can_read_path(&fixture.path("restricted")));
        assert!(!current_client_can_read_path(
            &fixture.path(r"restricted\hidden.bin")
        ));
    });
    assert!(matches!(
        fixture.request_with_restricted_contexts(SearchRequest::GetStatus),
        SearchResponse::Error(reason) if reason == "Authorization failed"
    ));
    assert!(matches!(
        fixture.request_with_restricted_contexts(SearchRequest::FolderSize {
            path: fixture.path("")
        }),
        SearchResponse::Error(reason) if reason == "Authorization failed"
    ));
    drop(access);
    with_restricting_sid(|| assert!(!current_client_can_read_path(&fixture.path(""))));
}

#[test]
fn snapshot_rejects_foreign_client_for_unreadable_folder() {
    let fixture = EndpointFixture::new();
    let _access = RestrictedRead::root(&fixture.root);
    assert!(matches!(
        fixture.request_with_restricted_contexts(SearchRequest::FolderSize { path: fixture.path("restricted") }),
        SearchResponse::Error(reason) if reason == "Authorization failed"
    ));
}

#[test]
fn snapshot_rejects_query_despite_readable_metadata() {
    let fixture = EndpointFixture::new();
    let _root_access = RestrictedRead::root(&fixture.root);
    let _attributes = RestrictedRead::attributes(&fixture.root, "public.bin");
    with_restricting_sid(|| {
        assert!(std::fs::metadata(fixture.path("public.bin")).is_ok());
        assert!(!current_client_can_read_path(&fixture.path("public.bin")));
    });
    assert!(matches!(
        fixture.request_with_restricted_contexts(SearchRequest::Query {
            text: "public.bin".into(), offset: 0, limit: 10,
        }),
        SearchResponse::Error(reason) if reason == "Authorization failed"
    ));
}

#[test]
fn snapshot_rejects_activity_query_despite_readable_ancestor() {
    let fixture = EndpointFixture::new();
    let _root_access = RestrictedRead::root(&fixture.root);
    assert!(fixture.index.write().update_file_metrics(
        fixture.hidden_frn,
        FileMetrics {
            logical_size: 54,
            allocated_size: 54
        },
        std::time::Instant::now(),
    ));
    assert!(matches!(
        fixture.request_with_restricted_contexts(SearchRequest::CheckPathsModified {
            paths: vec![fixture.path(""), fixture.path("restricted")], threshold_secs: 60,
        }),
        SearchResponse::Error(reason) if reason == "Authorization failed"
    ));
}

#[test]
fn synthetic_restricted_read_grant_is_restored_on_unwind() {
    let fixture = EndpointFixture::new();
    with_restricting_sid(|| assert!(!current_client_can_read_path(&fixture.path(""))));
    let access = RestrictedRead::root(&fixture.root);
    with_restricting_sid(|| assert!(current_client_can_read_path(&fixture.path(""))));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let _access = access;
        panic!("synthetic permission-fixture unwind");
    }));
    assert!(result.is_err());
    with_restricting_sid(|| assert!(!current_client_can_read_path(&fixture.path(""))));
}
