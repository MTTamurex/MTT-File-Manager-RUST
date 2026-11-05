use super::{
    inspect_client_with, query_client_image_with_fallback, verify_sibling_directories,
    ClientIdentityError,
};
use std::path::PathBuf;

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
    ) {
        Err(ClientIdentityError::Unavailable(reason)) => {
            assert!(reason.contains("service context unavailable"));
            assert!(reason.contains("client context unavailable"));
        }
        _ => panic!("failed inspection must remain explicitly unavailable"),
    }
}

#[test]
fn missing_service_image_is_unavailable_after_a_matching_basename() {
    assert!(matches!(
        inspect_client_with(
            || Ok(r"Z:\Example\Install\mtt-file-manager.exe".into()),
            || panic!("successful inspection must not retry"),
            || Err("service image unavailable".into()),
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
