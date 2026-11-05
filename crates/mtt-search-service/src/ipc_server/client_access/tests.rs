use super::{authorize_request_with, ClientAuthorization, ClientIdentityError};

#[test]
fn trusted_folder_size_client_does_not_use_acl_fallback() {
    assert_eq!(
        authorize_request_with(Ok(()), Some(r"C:\Example"), |_| panic!(
            "trusted client needs no ACL fallback"
        )),
        Ok(ClientAuthorization::TrustedClient),
    );
}

#[test]
fn rejected_identity_never_uses_a_readable_folder_as_fallback() {
    let result = authorize_request_with(
        Err(ClientIdentityError::Rejected("foreign image".into())),
        Some(r"C:\Example"),
        |_| panic!("a known rejection must not reach ACL authorization"),
    );
    assert_eq!(result, Err("foreign image".into()));
}

#[test]
fn rejected_identity_also_blocks_search_and_modification_checks() {
    assert!(authorize_request_with(
        Err(ClientIdentityError::Rejected("foreign image".into())),
        None,
        |_| panic!("not a folder request"),
    )
    .is_err());
}

#[test]
fn readable_folder_size_path_recovers_unavailable_client_image_check() {
    assert!(matches!(
        authorize_request_with(
            Err(ClientIdentityError::Unavailable(
                "client process unavailable".into()
            )),
            Some(r"C:\Example"),
            |path| {
                assert_eq!(path, r"C:\Example");
                Ok(true)
            },
        ),
        Ok(ClientAuthorization::LegacyPathChecks { .. })
    ));
}

#[test]
fn unreadable_folder_size_path_remains_fail_closed() {
    let error = authorize_request_with(
        Err(ClientIdentityError::Unavailable(
            "client process unavailable".into(),
        )),
        Some(r"C:\Example"),
        |_| Ok(false),
    )
    .unwrap_err();
    assert!(error.contains("client cannot read requested path"));
}

#[test]
fn failed_folder_size_impersonation_remains_fail_closed() {
    let error = authorize_request_with(
        Err(ClientIdentityError::Unavailable(
            "client process unavailable".into(),
        )),
        Some(r"C:\Example"),
        |_| Err("impersonation unavailable".into()),
    )
    .unwrap_err();
    assert!(error.contains("ACL fallback failed"));
}

#[test]
fn unavailable_search_identity_defers_to_existing_handler_path_checks() {
    assert!(matches!(
        authorize_request_with(
            Err(ClientIdentityError::Unavailable(
                "client process unavailable".into()
            )),
            None,
            |_| panic!("path checks run inside the search/modification handler"),
        ),
        Ok(ClientAuthorization::LegacyPathChecks { .. })
    ));
}

#[test]
fn error_text_cannot_promote_rejection_to_unavailable() {
    assert!(authorize_request_with(
        Err(ClientIdentityError::Rejected(
            "client process unavailable".into()
        )),
        Some(r"C:\Example"),
        |_| panic!("classification must not parse the message text"),
    )
    .is_err());
}
