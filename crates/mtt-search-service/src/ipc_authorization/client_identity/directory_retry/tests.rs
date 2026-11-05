use windows::Win32::Foundation::HANDLE;

use super::super::os_fixture::{assert_no_thread_token, with_restricting_sid, IdentityPipe};
use super::super::{
    inspect_client_with, verify_sibling_directories, ClientIdentityError, PipeImpersonationGuard,
};
use super::{open_directory, verify_as_client, verify_with_anchor};

const IMPERSONATION: u32 = 0x00020000;
const IDENTIFICATION: u32 = 0x00010000;

fn fixture() -> tempfile::TempDir {
    tempfile::tempdir().unwrap_or_else(|_| panic!("cannot create directory-retry fixture"))
}

fn assert_unavailable(result: Result<(), ClientIdentityError>) {
    assert!(matches!(result, Err(ClientIdentityError::Unavailable(_))));
}

#[test]
fn anchored_alias_is_recovered_using_actual_client_impersonation() {
    let root = fixture();
    let alias = root.path().join(".");
    let anchor = open_directory(root.path()).unwrap();
    let pipe = IdentityPipe::new(IMPERSONATION);
    assert_eq!(verify_as_client(pipe.raw(), root.path(), &alias), Ok(()));
    assert_no_thread_token();
    with_restricting_sid(|| {
        assert_unavailable(verify_sibling_directories(root.path(), &alias));
        assert_eq!(verify_with_anchor(pipe.raw(), &anchor, &alias), Ok(()));
    });
    assert_no_thread_token();
}

#[test]
fn successful_resolution_of_a_different_directory_is_rejected() {
    let root = fixture();
    let other = root.path().join("other");
    std::fs::create_dir(&other).unwrap();
    let anchor = open_directory(root.path()).unwrap();
    let pipe = IdentityPipe::new(IMPERSONATION);
    let result = with_restricting_sid(|| {
        assert_unavailable(verify_sibling_directories(root.path(), &other));
        verify_with_anchor(pipe.raw(), &anchor, &other)
    });
    assert!(matches!(result, Err(ClientIdentityError::Rejected(_))));
    assert_no_thread_token();
}

#[test]
fn unavailable_service_anchor_cannot_be_recreated_under_client_context() {
    let root = fixture();
    let alias = root.path().join(".");
    let pipe = IdentityPipe::new(IMPERSONATION);
    with_restricting_sid(|| {
        assert_unavailable(verify_sibling_directories(root.path(), &alias));
        assert_unavailable(verify_as_client(pipe.raw(), root.path(), &alias));
    });
    assert_no_thread_token();
}

#[test]
fn missing_client_directory_stays_unavailable_even_with_a_valid_anchor() {
    let root = fixture();
    let anchor = open_directory(root.path()).unwrap();
    let pipe = IdentityPipe::new(IMPERSONATION);
    assert_unavailable(verify_with_anchor(
        pipe.raw(),
        &anchor,
        &root.path().join("missing"),
    ));
    assert_no_thread_token();
}

#[test]
fn identification_only_client_cannot_use_the_anchored_retry() {
    let root = fixture();
    let anchor = open_directory(root.path()).unwrap();
    let pipe = IdentityPipe::new(IDENTIFICATION);
    assert_unavailable(verify_with_anchor(pipe.raw(), &anchor, root.path()));
    assert_no_thread_token();
}

#[test]
fn actual_impersonation_failure_stays_unavailable_with_a_valid_anchor() {
    let root = fixture();
    let anchor = open_directory(root.path()).unwrap();
    assert_unavailable(verify_with_anchor(HANDLE::default(), &anchor, root.path()));
    assert_no_thread_token();
}

#[test]
fn a_file_cannot_be_used_as_the_service_directory_anchor() {
    let root = fixture();
    let file = root.path().join("file.txt");
    std::fs::write(&file, b"synthetic fixture").unwrap();
    let file = open_directory(&file).unwrap();
    let pipe = IdentityPipe::new(IMPERSONATION);
    assert_unavailable(verify_with_anchor(pipe.raw(), &file, root.path()));
    assert_no_thread_token();
}

#[test]
fn client_impersonation_guard_reverts_on_unwind() {
    let pipe = IdentityPipe::new(IMPERSONATION);
    let started = std::cell::Cell::new(false);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = PipeImpersonationGuard::new(pipe.raw()).unwrap();
        started.set(true);
        panic!("synthetic directory-retry unwind");
    }));
    assert!(result.is_err());
    assert!(
        started.get(),
        "setup failure cannot stand in for deliberate unwind"
    );
    assert_no_thread_token();
}

#[test]
fn pipeline_accepts_a_directory_retry_confirmed_by_actual_handles() {
    let root = fixture();
    let anchor = open_directory(root.path()).unwrap();
    let image = root.path().join(".").join("mtt-file-manager.exe");
    let service = root.path().join("mtt-search-service.exe");
    let pipe = IdentityPipe::new(IMPERSONATION);
    // The image is synthetic; directory failure and recovery use real Windows
    // calls. The anchor was obtained before restricting the initial test context.
    let result = with_restricting_sid(|| {
        inspect_client_with(
            || Ok(image.to_str().unwrap().to_owned()),
            || panic!("image query succeeded"),
            || Ok(service),
            |service_dir, client_dir| {
                assert!(service_dir == root.path());
                verify_with_anchor(pipe.raw(), &anchor, client_dir)
            },
        )
    });
    assert_eq!(result, Ok(()));
    assert_no_thread_token();
}

#[test]
fn missing_service_directory_stays_unavailable() {
    let root = fixture();
    let pipe = IdentityPipe::new(IMPERSONATION);
    let missing = root.path().join("missing-service-directory");
    assert_unavailable(verify_as_client(pipe.raw(), &missing, root.path()));
    assert_no_thread_token();
}
