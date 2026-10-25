use super::*;
use crate::domain::{file_entry::FileEntry, list_columns::PropertyId};

fn media_properties() -> [PropertyId; 2] {
    [3, 4].map(|pid| PropertyId {
        fmtid: 0x64440490_4c8b_11d1_8b70_080036b11a03,
        pid,
    })
}

/// One second of silent PCM, generated rather than relying on an external media fixture.
fn write_media(path: &std::path::Path) {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&16036_u32.to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&8000_u32.to_le_bytes());
    bytes.extend_from_slice(&16000_u32.to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&16000_u32.to_le_bytes());
    bytes.resize(16044, 0);
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn media_length_and_bitrate_are_resolved_together_with_generic_shell_fallback() {
    let _com = ComGuard::new().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("sample.wav");
    write_media(&path);
    let item = FileEntry::from_path(path, false);
    let properties = media_properties();
    let request = DisplayRequest {
        keys: properties
            .iter()
            .map(|&property| ValueKey::new(&item, 1, property))
            .collect(),
        ctx: egui::Context::default(),
        cancelled: Default::default(),
    };
    let (sender, receiver) = bounded(8);
    assert!(read_request(
        request,
        &sender,
        &mut IndexedProperties::default()
    ));
    let values: Vec<_> = receiver.try_iter().collect();
    assert_eq!(values.len(), 2);
    for property in properties {
        let (_, value) = values
            .iter()
            .find(|(key, _)| key.property == property)
            .unwrap();
        assert!(!value
            .as_ref()
            .expect("local media metadata")
            .text()
            .is_empty());
    }
}

#[test]
fn absent_fast_properties_are_not_mistaken_for_missing_handler_properties() {
    let _com = ComGuard::new().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("sample.wav");
    write_media(&path);
    let duration = media_properties()[0];
    let fast = PropertyReader::open_fast(&path).unwrap();
    assert!(fast.read(duration).is_none());
    assert!(PropertyReader::open(&path)
        .unwrap()
        .read(duration)
        .is_some());
}

#[test]
fn independent_file_requests_complete_through_the_bounded_worker_pool() {
    let directory = tempfile::tempdir().unwrap();
    let (sender, receiver) = start();
    let sender = sender.unwrap();
    let property = media_properties()[0];
    for name in ["first.wav", "second.wav", "third.wav", "fourth.wav"] {
        let path = directory.path().join(name);
        write_media(&path);
        let item = FileEntry::from_path(path, false);
        sender
            .send(DisplayRequest {
                keys: vec![ValueKey::new(&item, 1, property)],
                ctx: egui::Context::default(),
                cancelled: Default::default(),
            })
            .unwrap();
    }
    let mut paths = std::collections::HashSet::new();
    for _ in 0..4 {
        let (key, value) = receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        assert!(!value.unwrap().text().is_empty());
        assert!(paths.insert(key.path));
    }
}

#[test]
fn missing_media_metadata_still_defers_while_the_file_is_locked_for_writing() {
    use std::os::windows::fs::OpenOptionsExt;
    let _com = ComGuard::new().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("sample.wav");
    write_media(&path);
    let _writer = std::fs::OpenOptions::new()
        .write(true)
        .share_mode(0)
        .open(&path)
        .unwrap();
    let item = FileEntry::from_path(path, false);
    let request = DisplayRequest {
        keys: vec![ValueKey::new(&item, 1, media_properties()[0])],
        ctx: egui::Context::default(),
        cancelled: Default::default(),
    };
    let (sender, receiver) = bounded(2);
    assert!(read_request(
        request,
        &sender,
        &mut IndexedProperties::default()
    ));
    assert!(receiver.try_recv().unwrap().1.is_none());
}

#[test]
fn cancelled_queue_entries_are_discarded_without_publishing_or_reading_properties() {
    let item = FileEntry::from_path("file.txt".into(), false);
    let request = DisplayRequest {
        keys: vec![ValueKey::new(&item, 1, media_properties()[0])],
        ctx: egui::Context::default(),
        cancelled: Arc::new(AtomicBool::new(true)),
    };
    let (sender, receiver) = bounded(2);
    // No COM initialization: a cancelled request must not reach either Shell or the index.
    assert!(read_request(
        request,
        &sender,
        &mut IndexedProperties::default()
    ));
    assert!(receiver.is_empty());
}
