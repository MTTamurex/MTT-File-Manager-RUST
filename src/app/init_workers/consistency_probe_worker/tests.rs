use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_temp_path(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("mtt_consistency_probe_{name}_{nanos}"))
}

#[test]
fn cover_scan_throttle_allows_first_scan_and_blocks_early_repeat() {
    let now = Instant::now();
    assert!(cover_scan_is_due(None, now));
    assert!(!cover_scan_is_due(Some(now), now + Duration::from_secs(5)));
    assert!(cover_scan_is_due(Some(now), now + COVER_SCAN_INTERVAL));
}

#[test]
fn liveness_check_accepts_existing_directory() {
    let path = unique_temp_path("dir");
    std::fs::create_dir(&path).unwrap();
    assert!(!directory_is_confirmed_gone(&path));
    let _ = std::fs::remove_dir(&path);
}

#[test]
fn liveness_check_detects_missing_directory() {
    let path = unique_temp_path("missing");
    assert!(directory_is_confirmed_gone(&path));
}

#[test]
fn liveness_check_rejects_file_replacing_directory() {
    let path = unique_temp_path("file");
    std::fs::write(&path, b"not a directory").unwrap();
    assert!(directory_is_confirmed_gone(&path));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn liveness_check_accepts_archive_root_file() {
    let path = unique_temp_path("archive").with_extension("zip");
    std::fs::write(&path, b"zip placeholder").unwrap();
    assert!(!directory_is_confirmed_gone(&path));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn liveness_check_accepts_path_inside_existing_archive() {
    let archive = unique_temp_path("archive_inner").with_extension("zip");
    std::fs::write(&archive, b"zip placeholder").unwrap();
    let inner_path = archive.join("folder");
    assert!(!directory_is_confirmed_gone(&inner_path));
    let _ = std::fs::remove_file(&archive);
}

#[test]
fn liveness_check_detects_path_inside_missing_archive() {
    let archive = unique_temp_path("missing_archive").with_extension("zip");
    let inner_path = archive.join("folder");
    assert!(directory_is_confirmed_gone(&inner_path));
}

#[test]
fn virtual_views_are_never_reported_as_missing_directories() {
    use crate::domain::special_paths::{tag_view_path, COMPUTER_VIEW_ID, RECYCLE_BIN_VIEW_ID};
    for id in 1..=5 {
        assert!(!directory_is_confirmed_gone(std::path::Path::new(
            &tag_view_path(id)
        )));
    }
    for path in [COMPUTER_VIEW_ID, RECYCLE_BIN_VIEW_ID] {
        assert!(!directory_is_confirmed_gone(std::path::Path::new(path)));
    }
}

#[test]
fn index_only_probe_acknowledges_without_a_directory_read_or_liveness_check() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing-folder");
    let (sender, receiver) = spawn_consistency_probe_worker(egui::Context::default());
    sender
        .send(ConsistencyProbeRequest {
            path: path.clone(),
            is_onedrive: false,
            ui_signature: 42,
            show_hidden_files: false,
            mode: ConsistencyProbeMode::FolderContents,
            modified_threshold_secs: 5,
            folder_cover_states: Vec::new(),
        })
        .unwrap();
    let result = receiver.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(result.path, path);
    assert_eq!(result.mode, ConsistencyProbeMode::FolderContents);
    assert_eq!(result.disk_signature, 42);
    // A listing/liveness probe would classify this nonexistent directory as gone.
    assert!(!result.path_vanished);
    assert!(result.changed_folder_contents.is_empty());
    assert!(result.changed_folder_covers.is_empty());
}
