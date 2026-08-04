use super::file_icons::{encode_png_lossless, file_icon_cache_bytes};
use super::file_icons_gc::is_on_accessible_drive;
use super::IconDiskCache;
use crate::domain::file_entry::IconSize;
use rusqlite::params;
use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::Ordering;

#[test]
fn file_icon_cache_round_trips_lossless_png() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cache = IconDiskCache::new(dir.path());
    let exe = dir.path().join("tool.exe");
    std::fs::write(&exe, b"v1").expect("write source file");
    let key = cache
        .file_icon_cache_key(&exe, IconSize::Jumbo)
        .expect("file icon key");
    let pixels = vec![
        255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 64, 255, 255, 255, 0,
    ];

    cache.save_file_icon(&key, &pixels, 2, 2);

    let (loaded, width, height) = cache.load_file_icon(&key).expect("cached icon");
    assert_eq!(loaded, pixels);
    assert_eq!(width, 2);
    assert_eq!(height, 2);
}

#[test]
fn in_memory_fallback_round_trips_within_session() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cache = IconDiskCache::in_memory_fallback();
    let exe = dir.path().join("tool.exe");
    std::fs::write(&exe, b"v1").expect("write source file");
    let key = cache
        .file_icon_cache_key(&exe, IconSize::Jumbo)
        .expect("file icon key");
    let pixels = vec![
        255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 64, 255, 255, 255, 0,
    ];

    assert!(cache.load_file_icon(&key).is_none());
    cache.save_file_icon(&key, &pixels, 2, 2);
    let (loaded, width, height) = cache.load_file_icon(&key).expect("cached icon");
    assert_eq!(loaded, pixels);
    assert_eq!(width, 2);
    assert_eq!(height, 2);
}

#[test]
fn file_icon_cache_key_changes_when_file_size_changes() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cache = IconDiskCache::new(dir.path());
    let exe = dir.path().join("tool.exe");
    std::fs::write(&exe, b"v1").expect("write first version");
    let key_v1 = cache
        .file_icon_cache_key(&exe, IconSize::Jumbo)
        .expect("first key");

    std::fs::write(&exe, b"version two").expect("write second version");
    let key_v2 = cache
        .file_icon_cache_key(&exe, IconSize::Jumbo)
        .expect("second key");

    assert_ne!(key_v1, key_v2);
}

#[test]
fn file_icon_cache_removes_orphaned_source_files_during_scan() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cache = IconDiskCache::new(dir.path());
    let exe = dir.path().join("tool.exe");
    std::fs::write(&exe, b"v1").expect("write source file");
    let key = cache
        .file_icon_cache_key(&exe, IconSize::Jumbo)
        .expect("file icon key");
    let pixels = vec![
        255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 64, 255, 255, 255, 0,
    ];

    cache.save_file_icon(&key, &pixels, 2, 2);
    assert!(cache.load_file_icon(&key).is_some());
    assert!(cache.file_icon_cache_bytes.load(Ordering::Relaxed) > 0);

    std::fs::remove_file(&exe).expect("delete source file");
    assert_eq!(cache.garbage_collect_file_icons(), 1);

    assert!(cache.load_file_icon(&key).is_none());
    assert_eq!(cache.file_icon_cache_bytes.load(Ordering::Relaxed), 0);
}

#[test]
fn saving_an_icon_does_not_scan_other_source_files() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cache = IconDiskCache::new(dir.path());
    let first_exe = dir.path().join("first.exe");
    let second_exe = dir.path().join("second.exe");
    std::fs::write(&first_exe, b"first").expect("write first source file");
    std::fs::write(&second_exe, b"second").expect("write second source file");
    let first_key = cache
        .file_icon_cache_key(&first_exe, IconSize::Jumbo)
        .expect("first icon key");
    let second_key = cache
        .file_icon_cache_key(&second_exe, IconSize::Jumbo)
        .expect("second icon key");
    let pixels = vec![255, 0, 0, 255];

    cache.save_file_icon(&first_key, &pixels, 1, 1);
    std::fs::remove_file(&first_exe).expect("delete first source file");
    cache.save_file_icon(&second_key, &pixels, 1, 1);

    assert!(cache.load_file_icon(&first_key).is_some());
    assert_eq!(cache.garbage_collect_file_icons(), 1);
    assert!(cache.load_file_icon(&first_key).is_none());
}

#[test]
fn gc_skips_unknown_or_unavailable_path_roots() {
    let accessible = HashSet::from(["C:\\".to_string()]);

    assert!(!is_on_accessible_drive(
        Path::new(r"\\server\share\tool.exe"),
        &accessible
    ));
    assert!(!is_on_accessible_drive(
        Path::new(r"\\?\E:\tools\tool.exe"),
        &accessible
    ));
    assert!(is_on_accessible_drive(
        Path::new(r"C:\tools\tool.exe"),
        &accessible
    ));
}

#[test]
fn late_corrupt_row_cleanup_does_not_delete_replacement() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cache = IconDiskCache::new(dir.path());
    let exe = dir.path().join("tool.exe");
    std::fs::write(&exe, b"v1").expect("write source file");
    let key = cache
        .file_icon_cache_key(&exe, IconSize::Jumbo)
        .expect("file icon key");
    let old_pixels = vec![255, 0, 0, 255];
    let new_pixels = vec![0, 255, 0, 255];
    let old_encoded = encode_png_lossless(&old_pixels, 1, 1).expect("encode old icon");
    let new_encoded = encode_png_lossless(&new_pixels, 1, 1).expect("encode new icon");

    cache.save_file_icon(&key, &old_pixels, 1, 1);
    let old_last_accessed = {
        let db = cache.file_icon_db.lock();
        let old_last_accessed = db
            .query_row(
                "SELECT last_accessed_at FROM file_icons WHERE id = ?",
                [&key.id],
                |row| row.get::<_, i64>(0),
            )
            .expect("old last access");
        db.execute(
            "UPDATE file_icons
             SET data = ?1, byte_len = ?2, last_accessed_at = ?3
             WHERE id = ?4",
            params![
                &new_encoded,
                new_encoded.len() as i64,
                old_last_accessed + 1,
                &key.id
            ],
        )
        .expect("replace icon row");
        old_last_accessed
    };
    cache.sync_file_icon_cache_bytes();
    cache.delete_file_icon_if_unchanged(&key.id, &old_encoded, 1, 1, old_last_accessed);

    let (loaded, _, _) = cache.load_file_icon(&key).expect("replacement icon");
    assert_eq!(loaded, new_pixels);
    let db_total = {
        let db = cache.file_icon_db.lock();
        file_icon_cache_bytes(&db).expect("cache size")
    };
    assert_eq!(
        cache.file_icon_cache_bytes.load(Ordering::Relaxed),
        db_total
    );
}

#[test]
fn invalid_stored_dimensions_remove_poisoned_row() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cache = IconDiskCache::new(dir.path());
    let exe = dir.path().join("tool.exe");
    std::fs::write(&exe, b"v1").expect("write source file");
    let key = cache
        .file_icon_cache_key(&exe, IconSize::Jumbo)
        .expect("file icon key");
    let pixels = vec![255, 0, 0, 255];

    cache.save_file_icon(&key, &pixels, 1, 1);
    {
        let db = cache.file_icon_db.lock();
        db.execute("UPDATE file_icons SET width = -1 WHERE id = ?", [&key.id])
            .expect("corrupt dimensions");
    }

    assert!(cache.load_file_icon(&key).is_none());
    let row_count = {
        let db = cache.file_icon_db.lock();
        db.query_row(
            "SELECT COUNT(*) FROM file_icons WHERE id = ?",
            [&key.id],
            |row| row.get::<_, i64>(0),
        )
        .expect("row count")
    };
    assert_eq!(row_count, 0);
    assert_eq!(cache.file_icon_cache_bytes.load(Ordering::Relaxed), 0);
}
