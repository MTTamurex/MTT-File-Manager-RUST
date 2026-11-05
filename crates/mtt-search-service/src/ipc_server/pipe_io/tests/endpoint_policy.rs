//! Known-foreign denials and compatibility with explicitly unavailable identity.
//! All disk data and ACL changes are confined to synthetic temporary fixtures.

use mtt_search_protocol::{SearchRequest, SearchResponse};

use crate::file_index::FileMetrics;
use crate::ipc_authorization::current_client_can_read_path;

use super::endpoint_acl::DenyRead;
use super::endpoint_fixture::EndpointFixture;
use super::restricted_token::without_admin_rights;

fn query(text: &str) -> SearchRequest {
    SearchRequest::Query {
        text: text.into(),
        offset: 0,
        limit: 10,
    }
}

#[test]
fn identified_foreign_client_is_denied_at_all_three_index_endpoints() {
    let fixture = EndpointFixture::new();
    assert!(matches!(
        fixture.request(SearchRequest::GetStatus),
        SearchResponse::Error(error) if error == "Authorization failed"
    ));
    for request in [
        query("public.bin"),
        query(""),
        SearchRequest::FolderSize {
            path: fixture.path(""),
        },
        SearchRequest::CheckPathsModified {
            paths: vec![fixture.path("")],
            threshold_secs: 60,
        },
    ] {
        assert!(matches!(
            fixture.request(request),
            SearchResponse::Error(error) if error == "Authorization failed"
        ));
    }
    assert!(matches!(
        fixture.request(SearchRequest::Ping),
        SearchResponse::Pong
    ));
}

#[test]
fn unavailable_identity_retains_readable_search_results() {
    let fixture = EndpointFixture::new();
    match fixture.request_with_unavailable_identity(query("public.bin")) {
        SearchResponse::Results {
            items,
            has_more,
            total_matches,
        } => {
            assert_eq!(items.len(), 1);
            assert!(items[0].full_path == fixture.path("public.bin"));
            assert_eq!(items[0].size, 11);
            assert!(!has_more);
            assert_eq!(total_matches, Some(1));
        }
        _ => panic!("unavailable identity must retain legacy ACL-authorized results"),
    }
}

#[test]
fn simulated_trusted_identity_keeps_all_three_handlers_functional_and_totals_complete() {
    let fixture = EndpointFixture::new();
    let _deny = DenyRead::new(&fixture.root, "restricted");
    let _deny_file = DenyRead::new(&fixture.root, r"restricted\hidden.bin");
    assert!(matches!(
        fixture.request_with_identity(query("public.bin"), Ok(())),
        SearchResponse::Results { items, total_matches: Some(1), .. }
            if items.len() == 1 && items[0].size == 11
    ));
    assert!(matches!(
        fixture.request_with_identity(
            SearchRequest::FolderSize {
                path: fixture.path("")
            },
            Ok(())
        ),
        SearchResponse::FolderSize {
            total_size: 101,
            file_count: 3,
            folder_count: 1,
            ..
        }
    ));
    assert!(fixture.index.write().update_file_metrics(
        fixture.hidden_frn,
        FileMetrics {
            logical_size: 54,
            allocated_size: 54
        },
        std::time::Instant::now(),
    ));
    assert!(matches!(
        fixture.request_with_identity(SearchRequest::CheckPathsModified {
            paths: vec![fixture.path("")], threshold_secs: 60,
        }, Ok(())),
        SearchResponse::PathsModified { modified } if modified == vec![fixture.path("")]
    ));
}

#[test]
fn unavailable_query_retains_parent_based_metadata_authorization() {
    let fixture = EndpointFixture::new();
    let _deny = DenyRead::new(&fixture.root, "blocked.bin");
    without_admin_rights(|| {
        assert!(current_client_can_read_path(&fixture.path("")));
        assert!(!current_client_can_read_path(&fixture.path("blocked.bin")));
        let error = std::fs::File::open(fixture.path("blocked.bin")).unwrap_err();
        assert_eq!(error.raw_os_error(), Some(5));
        // Attributes remain visible: this is not a denied-name/metadata case.
        assert!(std::fs::metadata(fixture.path("blocked.bin")).is_ok());
    });
    match fixture.request_with_unavailable_identity(query("blocked.bin")) {
        SearchResponse::Results {
            items,
            total_matches,
            ..
        } => {
            assert_eq!(items.len(), 1);
            assert!(items[0].full_path == fixture.path("blocked.bin"));
            assert_eq!(items[0].size, 37);
            assert_eq!(total_matches, Some(1));
        }
        _ => panic!("current parent-based search should return indexed metadata"),
    }
}

#[test]
fn unavailable_query_filters_denied_parent_and_rechecks_after_acl_restoration() {
    let fixture = EndpointFixture::new();
    let deny = DenyRead::new(&fixture.root, "restricted");
    without_admin_rights(|| {
        assert!(!current_client_can_read_path(&fixture.path("restricted")));
        assert_eq!(
            std::fs::read_dir(fixture.path("restricted"))
                .unwrap_err()
                .raw_os_error(),
            Some(5)
        );
    });
    assert!(matches!(
        fixture.request_with_unavailable_identity(query("hidden.bin")),
        SearchResponse::Results { items, has_more: false, total_matches: Some(0) }
            if items.is_empty()
    ));
    drop(deny);
    without_admin_rights(|| assert!(current_client_can_read_path(&fixture.path("restricted"))));
    assert!(matches!(
        fixture.request_with_unavailable_identity(query("hidden.bin")),
        SearchResponse::Results { items, total_matches: Some(1), .. }
            if items.len() == 1 && items[0].size == 53
    ));
}

#[test]
fn unavailable_folder_size_preserves_aggregates_but_denies_the_exact_denied_path() {
    let fixture = EndpointFixture::new();
    let _deny = DenyRead::new(&fixture.root, "restricted");
    let _deny_file = DenyRead::new(&fixture.root, r"restricted\hidden.bin");
    without_admin_rights(|| {
        assert!(current_client_can_read_path(&fixture.path("")));
        assert!(!current_client_can_read_path(&fixture.path("restricted")));
        assert!(!current_client_can_read_path(
            &fixture.path(r"restricted\hidden.bin")
        ));
    });
    match fixture.request_with_unavailable_identity(SearchRequest::FolderSize {
        path: fixture.path(""),
    }) {
        SearchResponse::FolderSize {
            path,
            total_size,
            file_count,
            folder_count,
        } => {
            assert!(path == fixture.path(""));
            assert_eq!(total_size, 11 + 37 + 53);
            assert_eq!(file_count, 3);
            assert_eq!(folder_count, 1);
        }
        _ => panic!("readable parent should return complete indexed subtree totals"),
    }
    assert!(matches!(
        fixture.request_with_unavailable_identity(SearchRequest::FolderSize { path: fixture.path("restricted") }),
        SearchResponse::Error(error) if error == "Authorization failed"
    ));
}

#[test]
fn unavailable_modification_check_preserves_ancestor_activity_and_path_authorization() {
    let fixture = EndpointFixture::new();
    let _deny = DenyRead::new(&fixture.root, "restricted");
    let _deny_file = DenyRead::new(&fixture.root, r"restricted\hidden.bin");
    without_admin_rights(|| {
        assert!(current_client_can_read_path(&fixture.path("")));
        assert!(!current_client_can_read_path(&fixture.path("restricted")));
        assert!(!current_client_can_read_path(
            &fixture.path(r"restricted\hidden.bin")
        ));
    });
    let request = || SearchRequest::CheckPathsModified {
        paths: vec![fixture.path(""), fixture.path("restricted")],
        threshold_secs: 60,
    };
    assert!(matches!(
        fixture.request_with_unavailable_identity(request()),
        SearchResponse::PathsModified { modified } if modified.is_empty()
    ));
    assert!(fixture.index.write().update_file_metrics(
        fixture.hidden_frn,
        FileMetrics {
            logical_size: 54,
            allocated_size: 54
        },
        std::time::Instant::now(),
    ));
    match fixture.request_with_unavailable_identity(request()) {
        SearchResponse::PathsModified { modified } => {
            assert!(modified == vec![fixture.path("")]);
        }
        _ => panic!("readable ancestor should expose its indexed modification activity"),
    }
}

#[test]
fn synthetic_deny_acl_is_restored_even_when_the_test_action_panics() {
    let fixture = EndpointFixture::new();
    let deny = DenyRead::new(&fixture.root, "blocked.bin");
    without_admin_rights(|| assert!(!current_client_can_read_path(&fixture.path("blocked.bin"))));
    // Setup assertions stay outside catch_unwind so a setup failure cannot be
    // mistaken for the deliberately triggered panic.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let _deny = deny;
        panic!("synthetic fixture unwind");
    }));
    assert!(result.is_err());
    without_admin_rights(|| assert!(current_client_can_read_path(&fixture.path("blocked.bin"))));
}
