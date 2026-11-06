use super::{
    inspect_client_with, query_client_image_with_fallback, verify_sibling_directories,
    ClientIdentityError,
};
use std::path::{Path, PathBuf};

fn no_directory_retry(_: &Path, _: &Path) -> Result<(), ClientIdentityError> {
    panic!("this identity outcome must not reach directory retry")
}

fn no_snapshot() -> Result<super::snapshot_rejection::BasenameEvidence, &'static str> {
    panic!("available image must never trigger process snapshot lookup")
}

struct Images {
    root: tempfile::TempDir,
    client: String,
    service: PathBuf,
}

fn images() -> Images {
    let root = tempfile::tempdir().unwrap();
    let client = root
        .path()
        .join("mtt-file-manager.exe")
        .to_str()
        .unwrap()
        .to_owned();
    let service = root.path().join("mtt-search-service.exe");
    Images {
        root,
        client,
        service,
    }
}

#[test]
fn trusted_sibling_remains_accepted_without_retry() {
    let images = images();
    assert_eq!(
        inspect_client_with(
            || Ok(images.client.clone()),
            || panic!("successful inspection must not retry"),
            || Ok(images.service.clone()),
            no_directory_retry,
            no_snapshot,
        ),
        Ok(())
    );
}

#[test]
fn trusted_sibling_remains_case_insensitive() {
    let images = images();
    assert_eq!(
        inspect_client_with(
            || Ok(images.client.to_ascii_uppercase()),
            || panic!("successful inspection must not retry"),
            || Ok(images.service.clone()),
            no_directory_retry,
            no_snapshot,
        ),
        Ok(())
    );
}

#[test]
fn impersonated_retry_can_recover_a_legitimate_client() {
    let images = images();
    assert_eq!(
        inspect_client_with(
            || Err("service context unavailable".into()),
            || Ok(images.client.clone()),
            || Ok(images.service.clone()),
            no_directory_retry,
            no_snapshot,
        ),
        Ok(())
    );
}

#[test]
fn foreign_basename_is_rejected_without_retry_or_service_lookup() {
    assert!(matches!(
        inspect_client_with(
            || Ok(r"Z:\Example\Install\other.exe".into()),
            || panic!("foreign image is not an inspection failure"),
            || panic!("known foreign basename needs no service lookup"),
            no_directory_retry,
            no_snapshot,
        ),
        Err(ClientIdentityError::Rejected(_))
    ));
}

#[test]
fn foreign_image_found_by_retry_is_still_rejected() {
    assert!(matches!(
        inspect_client_with(
            || Err("service context unavailable".into()),
            || Ok(r"Z:\Example\Install\other.exe".into()),
            || panic!("foreign retry needs no service lookup"),
            no_directory_retry,
            no_snapshot,
        ),
        Err(ClientIdentityError::Rejected(_))
    ));
}

#[test]
fn correct_basename_in_a_different_directory_is_rejected() {
    let images = images();
    std::fs::create_dir(images.root.path().join("other")).unwrap();
    let foreign = images
        .root
        .path()
        .join("other")
        .join("mtt-file-manager.exe");
    assert!(matches!(
        inspect_client_with(
            || Ok(foreign.to_str().unwrap().to_owned()),
            || panic!("a directory mismatch must not retry"),
            || Ok(images.service.clone()),
            no_directory_retry,
            no_snapshot,
        ),
        Err(ClientIdentityError::Rejected(_))
    ));
}

#[test]
fn unresolved_directory_mismatch_is_unavailable_not_confirmed_foreign() {
    let images = images();
    assert!(matches!(
        verify_sibling_directories(images.root.path(), &images.root.path().join("missing")),
        Err(ClientIdentityError::Unavailable(_))
    ));
    assert!(matches!(
        verify_sibling_directories(&images.root.path().join("missing"), images.root.path()),
        Err(ClientIdentityError::Unavailable(_))
    ));
}

#[test]
fn successful_legacy_lexical_comparison_is_preserved() {
    let images = images();
    let missing = images.root.path().join("missing");
    assert_eq!(verify_sibling_directories(&missing, &missing), Ok(()));
}

#[test]
fn rejection_reason_does_not_expose_client_paths_or_filenames() {
    let images = images();
    let client = images.root.path().join("other.exe");
    let error = inspect_client_with(
        || Ok(client.to_str().unwrap().to_owned()),
        || panic!("known foreign image must not retry"),
        || panic!("known foreign basename needs no service lookup"),
        no_directory_retry,
        no_snapshot,
    )
    .unwrap_err()
    .to_string();
    assert!(!error.contains("other.exe"));
    assert!(!error.contains(images.root.path().to_str().unwrap()));
}

#[test]
fn both_query_failures_are_unavailable_not_rejected_or_trusted() {
    match inspect_client_with(
        || Err("service context unavailable".into()),
        || Err("client context unavailable".into()),
        || panic!("failed inspection needs no service lookup"),
        no_directory_retry,
        || Err("snapshot unavailable"),
    ) {
        Err(ClientIdentityError::Unavailable(reason)) => {
            assert!(reason.contains("service context unavailable"));
            assert!(reason.contains("client context unavailable"));
        }
        _ => panic!("failed inspection must remain explicitly unavailable"),
    }
}

#[test]
fn snapshot_runs_once_after_both_queries_fail_and_never_grants_trust() {
    use super::snapshot_rejection::BasenameEvidence;
    for evidence in [
        Ok(BasenameEvidence::Foreign),
        Ok(BasenameEvidence::MatchesExpected),
        Err("snapshot unavailable"),
    ] {
        let foreign = evidence == Ok(BasenameEvidence::Foreign);
        let stage = std::cell::Cell::new(0);
        let result = inspect_client_with(
            || {
                assert_eq!(stage.replace(1), 0);
                Err("primary failure".into())
            },
            || {
                assert_eq!(stage.replace(2), 1);
                Err("retry failure".into())
            },
            || panic!("a snapshot must never reach service directory checks"),
            no_directory_retry,
            || {
                assert_eq!(stage.replace(3), 2);
                evidence
            },
        );
        assert_eq!(stage.get(), 3);
        match result {
            Err(ClientIdentityError::Rejected(_)) => assert!(foreign),
            Err(ClientIdentityError::Unavailable(reason)) => {
                assert!(!foreign);
                assert!(reason.contains("primary failure"));
                assert!(reason.contains("retry failure"));
            }
            Ok(()) => panic!("snapshot evidence must never authenticate the client"),
        }
    }
}

#[test]
fn missing_service_image_is_unavailable_after_a_matching_basename() {
    assert!(matches!(
        inspect_client_with(
            || Ok(r"Z:\Example\Install\mtt-file-manager.exe".into()),
            || panic!("successful inspection must not retry"),
            || Err("service image unavailable".into()),
            no_directory_retry,
            no_snapshot,
        ),
        Err(ClientIdentityError::Unavailable(_))
    ));
}

#[test]
fn client_image_query_uses_primary_result_without_impersonation() {
    let fallback_called = std::cell::Cell::new(false);
    let path = query_client_image_with_fallback(
        || Ok(r"C:\Program Files\MTT File Manager\mtt-file-manager.exe".into()),
        || {
            fallback_called.set(true);
            Err("fallback must not run".into())
        },
    )
    .unwrap();
    assert!(path.ends_with("mtt-file-manager.exe"));
    assert!(!fallback_called.get());
}

#[test]
fn client_image_query_retries_under_impersonation() {
    let path = query_client_image_with_fallback(
        || Err("service token denied".into()),
        || Ok(r"C:\Program Files\MTT File Manager\mtt-file-manager.exe".into()),
    )
    .unwrap();
    assert!(path.ends_with("mtt-file-manager.exe"));
}

#[test]
fn client_image_query_fails_when_both_security_contexts_fail() {
    let error = query_client_image_with_fallback(
        || Err("service token denied".into()),
        || Err("client token denied".into()),
    )
    .unwrap_err();
    assert!(error.contains("service token denied"));
    assert!(error.contains("client token denied"));
}

#[test]
fn unavailable_directory_comparison_retries_once_with_the_observed_paths() {
    let images = images();
    let parent = images.root.path().join("unresolved");
    let image = parent.join("mtt-file-manager.exe");
    let retried = std::cell::Cell::new(false);
    let result = inspect_client_with(
        || Ok(image.to_str().unwrap().to_owned()),
        || panic!("successful image query must not retry"),
        || Ok(images.service.clone()),
        |service_dir, client_dir| {
            assert!(!retried.replace(true));
            assert!(service_dir == images.root.path());
            assert!(client_dir == parent);
            Ok(())
        },
        no_snapshot,
    );
    assert_eq!(result, Ok(()));
    assert!(retried.get());
}

#[test]
fn confirmed_directory_mismatch_on_retry_is_rejected_not_unavailable() {
    let images = images();
    let image = images
        .root
        .path()
        .join("unresolved")
        .join("mtt-file-manager.exe");
    assert_eq!(
        inspect_client_with(
            || Ok(image.to_str().unwrap().to_owned()),
            || panic!("successful image query must not retry"),
            || Ok(images.service.clone()),
            |_, _| Err(ClientIdentityError::Rejected(
                "confirmed directory mismatch".into()
            )),
            no_snapshot,
        ),
        Err(ClientIdentityError::Rejected(
            "confirmed directory mismatch".into()
        ))
    );
}

#[test]
fn directory_retry_failure_keeps_both_context_reasons_and_stays_unavailable() {
    let images = images();
    let image = images
        .root
        .path()
        .join("unresolved")
        .join("mtt-file-manager.exe");
    match inspect_client_with(
        || Ok(image.to_str().unwrap().to_owned()),
        || panic!("successful image query must not retry"),
        || Ok(images.service.clone()),
        |_, _| {
            Err(ClientIdentityError::Unavailable(
                "synthetic retry failure".into(),
            ))
        },
        no_snapshot,
    ) {
        Err(ClientIdentityError::Unavailable(reason)) => {
            assert!(reason.contains("directory verification unavailable as service:"));
            assert!(reason.contains("client retry unavailable: synthetic retry failure"));
            assert!(!reason.contains(images.root.path().to_str().unwrap()));
        }
        _ => panic!("failed directory retry must remain unavailable"),
    }
}
