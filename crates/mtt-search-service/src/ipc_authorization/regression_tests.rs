//! In-memory compatibility tests; no processes, pipes, ACL edits or disk access.
//! These characterize current pagination, not a new metadata access policy.

use super::{
    collect_existing_authorized_page, file_index, parent_dir_of, AuthorizedPageRequest,
    CLIENT_CONNECTION_CHECK_INTERVAL,
};

fn result(name: &str, parent: &str) -> file_index::SearchResult {
    file_index::SearchResult {
        name: name.to_owned(),
        full_path: format!(r"{parent}\{name}"),
        is_dir: false,
        size: 42,
    }
}

#[test]
fn denied_candidates_do_not_consume_offset_or_page_capacity() {
    let items = vec![
        result("file.txt", r"C:\Restricted"),
        result("first.txt", r"C:\Data"),
        result("file.txt", r"C:\Restricted"),
        result("second.txt", r"C:\Data"),
    ];
    let page = collect_existing_authorized_page(
        items,
        AuthorizedPageRequest {
            raw_has_more: false,
            offset: 1,
            limit: 1,
        },
        |_| true,
        |parent| parent == r"C:\Data",
        |_| panic!("parent authorization should handle these paths"),
        |_| Ok(true),
    )
    .unwrap();

    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].name, "second.txt");
    assert_eq!(page.items[0].size, 42);
    assert!(!page.has_more);
    assert_eq!(page.total_matches, Some(2));
}

#[test]
fn denied_candidates_after_a_full_page_do_not_claim_another_page() {
    let page = collect_existing_authorized_page(
        vec![
            result("file.txt", r"C:\Data"),
            result("file.txt", r"C:\Restricted"),
        ],
        AuthorizedPageRequest {
            raw_has_more: false,
            offset: 0,
            limit: 1,
        },
        |_| true,
        |parent| parent == r"C:\Data",
        |_| false,
        |_| Ok(true),
    )
    .unwrap();

    assert_eq!(page.items.len(), 1);
    assert!(!page.has_more);
    assert_eq!(page.total_matches, Some(1));
}

#[test]
fn another_authorized_candidate_keeps_pagination_open() {
    let page = collect_existing_authorized_page(
        vec![
            result("first.txt", r"C:\Data"),
            result("second.txt", r"C:\Data"),
        ],
        AuthorizedPageRequest {
            raw_has_more: false,
            offset: 0,
            limit: 1,
        },
        |_| true,
        |_| true,
        |_| false,
        |_| Ok(true),
    )
    .unwrap();

    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].name, "first.txt");
    assert!(page.has_more);
    assert_eq!(page.total_matches, None);
}

#[test]
fn raw_scan_cap_without_authorized_items_does_not_cause_retry_loop() {
    let page = collect_existing_authorized_page(
        vec![result("file.txt", r"C:\Restricted")],
        AuthorizedPageRequest {
            raw_has_more: true,
            offset: 0,
            limit: 10,
        },
        |_| true,
        |_| false,
        |_| false,
        |_| Ok(true),
    )
    .unwrap();

    assert!(page.items.is_empty());
    assert!(!page.has_more);
    assert_eq!(page.total_matches, None);
}

#[test]
fn raw_scan_cap_with_results_allows_the_client_to_advance() {
    let page = collect_existing_authorized_page(
        vec![result("file.txt", r"C:\Data")],
        AuthorizedPageRequest {
            raw_has_more: true,
            offset: 0,
            limit: 10,
        },
        |_| true,
        |_| true,
        |_| false,
        |_| Ok(true),
    )
    .unwrap();

    assert_eq!(page.items.len(), 1);
    assert!(page.has_more);
    assert_eq!(page.total_matches, None);
}

#[test]
fn deadline_keeps_collected_results_and_stops_authorization_work() {
    let candidate_count = CLIENT_CONNECTION_CHECK_INTERVAL * 2;
    let items =
        (0..candidate_count).map(|index| result(&format!("file{index}.txt"), r"C:\Data"));
    let mut checked = 0;
    let mut authorized = 0;
    let page = collect_existing_authorized_page(
        items,
        AuthorizedPageRequest {
            raw_has_more: false,
            offset: 0,
            limit: candidate_count,
        },
        |_| {
            checked += 1;
            true
        },
        |_| {
            authorized += 1;
            true
        },
        |_| false,
        |index| Ok(index < CLIENT_CONNECTION_CHECK_INTERVAL),
    )
    .unwrap();

    assert_eq!(checked, CLIENT_CONNECTION_CHECK_INTERVAL);
    assert_eq!(authorized, CLIENT_CONNECTION_CHECK_INTERVAL);
    assert_eq!(page.items.len(), CLIENT_CONNECTION_CHECK_INTERVAL);
    assert!(page.has_more);
    assert_eq!(page.total_matches, None);
}

#[test]
fn disconnected_client_stops_before_filesystem_authorization() {
    let response = collect_existing_authorized_page(
        vec![result("file.txt", r"C:\Data")],
        AuthorizedPageRequest {
            raw_has_more: false,
            offset: 0,
            limit: 10,
        },
        |_| panic!("disconnected client must not trigger path checks"),
        |_| panic!("disconnected client must not trigger parent checks"),
        |_| panic!("disconnected client must not trigger access checks"),
        |_| Err("Client disconnected".to_owned()),
    );

    assert!(matches!(response, Err(error) if error == "Client disconnected"));
}

#[test]
fn disconnected_client_does_not_receive_a_partially_collected_page() {
    let candidate_count = CLIENT_CONNECTION_CHECK_INTERVAL * 2;
    let items =
        (0..candidate_count).map(|index| result(&format!("file{index}.txt"), r"C:\Data"));
    let mut checked = 0;
    let response = collect_existing_authorized_page(
        items,
        AuthorizedPageRequest {
            raw_has_more: false,
            offset: 0,
            limit: candidate_count,
        },
        |_| {
            checked += 1;
            true
        },
        |_| true,
        |_| false,
        |index| {
            if index == 0 {
                Ok(true)
            } else {
                Err("Client disconnected".to_owned())
            }
        },
    );

    assert_eq!(checked, CLIENT_CONNECTION_CHECK_INTERVAL);
    assert!(matches!(response, Err(error) if error == "Client disconnected"));
}

#[test]
fn candidate_without_parent_uses_exact_path_authorization() {
    for allowed in [false, true] {
        let mut checked_paths = Vec::new();
        let page = collect_existing_authorized_page(
            vec![file_index::SearchResult {
                name: "file.txt".to_owned(),
                full_path: "file.txt".to_owned(),
                is_dir: false,
                size: 42,
            }],
            AuthorizedPageRequest {
                raw_has_more: false,
                offset: 0,
                limit: 1,
            },
            |_| true,
            |_| panic!("a path without a parent needs exact-path authorization"),
            |path| {
                checked_paths.push(path.to_owned());
                allowed
            },
            |_| Ok(true),
        )
        .unwrap();

        assert_eq!(checked_paths, vec!["file.txt".to_owned()]);
        assert_eq!(page.items.len(), usize::from(allowed));
        assert_eq!(page.total_matches, Some(u32::from(allowed)));
        assert!(!page.has_more);
    }
}

#[test]
fn drive_root_parent_keeps_its_separator() {
    assert_eq!(parent_dir_of(r"C:\file.txt"), Some(r"C:\"));
    assert_eq!(parent_dir_of(r"C:\Data\file.txt"), Some(r"C:\Data"));
    assert_eq!(parent_dir_of("file.txt"), None);
}
