use super::*;
use rusqlite::params;
use std::time::{Duration, UNIX_EPOCH};

#[test]
fn existing_normal_database_rows_and_keys_survive_the_backend_change() {
    let directory = tempfile::tempdir().unwrap();
    let db_path = directory.path().join("thumbnails.db");
    let source = Path::new(r"C:\Videos\clip.mkv");
    let id = blake3::hash(source.as_os_str().as_encoded_bytes()).to_hex()[..32].to_string();
    let data = webp::Encoder::from_rgba(&[255; 16], 2, 2)
        .encode(80.0)
        .to_vec();
    {
        // The existing production layout, seeded without the promoted constructor.
        let db = Connection::open(&db_path).unwrap();
        db.execute_batch(
            "CREATE TABLE thumbnails (
            id TEXT PRIMARY KEY, path TEXT, data BLOB, modified_at INTEGER,
            created_at INTEGER, width INTEGER, height INTEGER, requested_size INTEGER
        );",
        )
        .unwrap();
        db.execute(
            "INSERT INTO thumbnails VALUES (?1, ?2, ?3, 42, 1, 2, 2, 512)",
            params![id, source.to_string_lossy(), data],
        )
        .unwrap();
    }
    let cache = ThumbnailDiskCache::new(directory.path().to_path_buf()).unwrap();
    assert!(cache.is_on_primary_path());
    assert_eq!(cache.primary_database_path(), db_path);
    let entry = cache
        .get(source, UNIX_EPOCH + Duration::from_secs(42))
        .unwrap();
    assert_eq!(entry.data, data);
    assert_eq!(
        (entry.width, entry.height, entry.requested_size),
        (2, 2, 512)
    );
    assert!(entry.satisfies_request(512));
    assert!(!directory.path().join("mpv-poc-v3").exists());
}

#[test]
fn experimental_stores_are_not_imported_or_modified() {
    let directory = tempfile::tempdir().unwrap();
    for name in ["mpv-poc-v1", "mpv-poc-v2", "mpv-poc-v3"] {
        let path = directory.path().join(name);
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("thumbnails.db"), b"untouched experimental store").unwrap();
    }
    let cache = ThumbnailDiskCache::new(directory.path().to_path_buf()).unwrap();
    assert_eq!(
        cache.primary_database_path(),
        directory.path().join("thumbnails.db")
    );
    assert!(cache.get_latest(Path::new(r"C:\Videos\clip.mkv")).is_none());
    for name in ["mpv-poc-v1", "mpv-poc-v2", "mpv-poc-v3"] {
        let path = directory.path().join(name);
        assert_eq!(
            std::fs::read(path.join("thumbnails.db")).unwrap(),
            b"untouched experimental store"
        );
        assert_eq!(std::fs::read_dir(path).unwrap().count(), 1);
    }
}
