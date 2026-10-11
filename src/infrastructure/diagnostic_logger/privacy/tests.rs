use super::sanitize_log_message;

#[test]
fn sanitizes_quoted_and_spaced_windows_paths() {
    let quoted = sanitize_log_message(
        "failed to open 'C:\\Users\\user\\Documents\\Project Name\\sample clip.mp4': access denied",
    );
    assert!(quoted.contains("<redacted>"));
    assert!(quoted.contains("access denied"));
    assert!(!quoted.contains("user"));
    assert!(!quoted.contains("Project Name"));
    assert!(!quoted.contains("sample clip.mp4"));
    assert_eq!(sanitize_log_message(&quoted), quoted);

    let unquoted = sanitize_log_message(
        "failed to open C:\\Users\\user\\Documents\\Project Name\\sample clip.mp4: access denied",
    );
    assert_eq!(unquoted, "failed to open <path>");
    assert_eq!(sanitize_log_message(&unquoted), unquoted);
    assert!(!unquoted.contains("user"));
    assert!(!unquoted.contains("Project Name"));
    assert!(!unquoted.contains("sample clip.mp4"));
}

#[test]
fn sanitizes_network_posix_and_relative_paths() {
    let network =
        sanitize_log_message(r"failed to read \\server\share\Sample Folder\sample.pdf: error");
    assert_eq!(network, "failed to read <path>");

    let posix =
        sanitize_log_message("failed to read /home/user/Documents/Project/sample.txt: error");
    assert_eq!(posix, "failed to read <path>");

    let relative = sanitize_log_message("failed to read Project Name/subfolder/sample.txt");
    assert_eq!(relative, "<path>");
}

#[test]
fn escaped_quotes_do_not_expose_quoted_path_fragments() {
    let message = sanitize_log_message(
        r#"failed for "C:\Users\user\Documents\Project \"sample\".txt"; retrying"#,
    );
    assert!(!message.contains("Users"));
    assert!(!message.contains("sample"));
    assert!(message.contains("retrying"));
}

#[test]
fn sanitizes_drive_names_labels_and_sensitive_key_values() {
    assert_eq!(
        sanitize_log_message("mounted drive D: Sample Drive Label"),
        "<drive>"
    );
    assert_eq!(
        sanitize_log_message("refresh failed on E:"),
        "refresh failed on <drive>"
    );
    assert_eq!(
        sanitize_log_message("enumeration failed for folder Sample Folder"),
        "<redacted>"
    );
    assert_eq!(
        sanitize_log_message("query=sample search result=2"),
        "query=<redacted>"
    );
    assert_eq!(
        sanitize_log_message("volume_label=Sample Volume"),
        "volume_label=<redacted>"
    );
    assert_eq!(
        sanitize_log_message("drive_type=removable drive_count=2"),
        "drive_type=removable drive_count=2"
    );
    assert_eq!(
        sanitize_log_message("path:Project Name/sample.txt"),
        "path:<redacted>"
    );
}

#[test]
fn sanitizes_common_machine_and_account_identifiers() {
    for value in [
        "user@example.invalid",
        "192.0.2.12",
        "00-11-22-33-44-55",
        "S-1-5-21-1234-5678-9012-3456",
        "12345678-1234-5678-9abc-def012345678",
        "fe80::1",
    ] {
        let message = sanitize_log_message(&format!("identifier {value} remains"));
        assert!(
            !message.contains(value),
            "identifier was retained: {message}"
        );
        assert!(message.contains("remains"));
    }
}

#[test]
fn redacts_free_text_containing_a_filename_without_leaking_its_prefix() {
    let message =
        sanitize_log_message("thumbnail decode failed code=17 for Sample Archive 2026.pdf");
    assert_eq!(message, "<file>");

    let technical = sanitize_log_message("thumbnail decode failed code=17 retry=1");
    assert_eq!(technical, "thumbnail decode failed code=17 retry=1");
}
