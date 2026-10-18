/// Add variant storage without rewriting or discarding the existing preview cache.
pub(crate) fn initialize_variant_table(conn: &rusqlite::Connection) {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS folder_preview_variants (
            folder_path TEXT NOT NULL,
            data BLOB NOT NULL,
            width INTEGER NOT NULL,
            height INTEGER NOT NULL,
            bucket_size INTEGER NOT NULL DEFAULT 256,
            format_version INTEGER NOT NULL,
            created_at INTEGER NOT NULL,
            PRIMARY KEY (folder_path, format_version)
        )",
        [],
    )
    .unwrap_or_else(|error| {
        log::warn!("[FOLDER PREVIEW CACHE] Failed to create variant table: {error}");
        0
    });
    // Existing rows remain intact but must be recomposed once before source-validated reuse.
    let _ = conn.execute(
        "ALTER TABLE folder_preview_variants ADD COLUMN source_fingerprint TEXT",
        [],
    );
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::domain::folder_appearance::FolderAppearance;

    fn put_variants(cache: &ThumbnailDiskCache, folder: &Path) {
        for appearance in FolderAppearance::ALL {
            let pixels = [20, 80, 120, 255].repeat(16 * 16);
            cache.put_folder_preview_for_appearance(folder, 16, &pixels, 16, 16, appearance);
        }
    }

    fn assert_absent(cache: &ThumbnailDiskCache, folder: &Path) {
        for appearance in FolderAppearance::ALL {
            assert!(cache
                .get_folder_preview_for_appearance(folder, 16, appearance)
                .is_none());
        }
    }

    #[test]
    fn folder_appearance_changes_preserve_independent_variants_at_different_resolutions() {
        use FolderAppearance::{BlueGray, DarkSlateBlue, Lavender, Original, PastelYellow};
        let dir = tempfile::tempdir().unwrap();
        let cache = ThumbnailDiskCache::new(dir.path().to_path_buf()).unwrap();
        let folder = dir.path().join("folder");
        std::fs::create_dir(&folder).unwrap();
        let variants = [
            (Original, 128, [240, 180, 50, 255]),
            (PastelYellow, 64, [232, 216, 138, 255]),
            (BlueGray, 32, [143, 175, 196, 255]),
            (DarkSlateBlue, 16, [70, 93, 112, 255]),
            (Lavender, 8, [181, 166, 203, 255]),
            (Original, 4, [240, 180, 50, 255]),
        ];
        for (appearance, size, color) in variants {
            let pixels = color.repeat(size as usize * size as usize);
            cache.put_folder_preview_for_appearance(&folder, size, &pixels, size, size, appearance);
        }
        for (appearance, size, color) in variants {
            let expected = color.repeat(size as usize * size as usize);
            let (decoded, _, _, _) = cache
                .get_folder_preview_for_appearance(&folder, size, appearance)
                .expect("switching colors must reuse the matching variant");
            assert_eq!(decoded, expected);
        }
    }

    #[test]
    fn folder_preview_variants_survive_reopening_cache() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("folder");
        std::fs::create_dir(&folder).unwrap();
        {
            let cache = ThumbnailDiskCache::new(dir.path().to_path_buf()).unwrap();
            put_variants(&cache, &folder);
        }
        let cache = ThumbnailDiskCache::new(dir.path().to_path_buf()).unwrap();
        for appearance in FolderAppearance::ALL {
            assert!(cache
                .get_folder_preview_for_appearance(&folder, 16, appearance)
                .is_some());
        }
    }

    #[test]
    fn folder_preview_invalidation_removes_all_variants() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("folder");
        std::fs::create_dir(&folder).unwrap();
        let cache = ThumbnailDiskCache::new(dir.path().to_path_buf()).unwrap();
        put_variants(&cache, &folder);
        cache.remove_folder_preview_cache(&folder);
        assert_absent(&cache, &folder);
        put_variants(&cache, &folder);
        cache.remove_cache_for_path(&folder);
        assert_absent(&cache, &folder);
    }

    #[test]
    fn folder_preview_gc_collects_variants_without_legacy_rows() {
        let dir = tempfile::tempdir().unwrap();
        let cache = ThumbnailDiskCache::new(dir.path().to_path_buf()).unwrap();
        let folder = dir.path().join("missing-folder");
        put_variants(&cache, &folder);
        assert_eq!(cache.garbage_collect(), FolderAppearance::ALL.len());
        assert_absent(&cache, &folder);
        put_variants(&cache, &folder);
        // A single folder means rowid sampling always includes at least one variant.
        assert_eq!(
            cache.garbage_collect_incremental(100),
            FolderAppearance::ALL.len()
        );
        assert_absent(&cache, &folder);
    }

    #[test]
    fn folder_preview_reads_legacy_cached_artwork_without_recomposition() {
        let dir = tempfile::tempdir().unwrap();
        let cache = ThumbnailDiskCache::new(dir.path().to_path_buf()).unwrap();
        let folder = dir.path().join("folder");
        let pixels = [230, 210, 140, 255].repeat(16 * 16);
        let data = webp::Encoder::from_rgba(&pixels, 16, 16)
            .encode_lossless()
            .to_vec();
        let appearance = FolderAppearance::PastelYellow;
        cache.writer.lock().execute(
            "INSERT INTO folder_previews (folder_path, data, width, height, bucket_size, format_version, created_at)
             VALUES (?, ?, 16, 16, 16, ?, 1)",
            params![folder.to_string_lossy(), data, appearance.cache_version(FOLDER_PREVIEW_CACHE_FORMAT_VERSION)],
        ).unwrap();
        let (decoded, _, _, _) = cache
            .get_folder_preview_for_appearance(&folder, 16, appearance)
            .unwrap();
        assert_eq!(decoded, pixels);
        put_variants(&cache, &folder);
        let count: i64 = cache
            .reader
            .lock()
            .query_row("SELECT COUNT(*) FROM folder_previews", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1, "legacy cache must remain intact");
    }
}
