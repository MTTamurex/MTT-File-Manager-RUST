use super::{FolderPreviewSource, ThumbnailDiskCache, FOLDER_PREVIEW_CACHE_FORMAT_VERSION};
use crate::domain::folder_appearance::FolderAppearance;
use crate::domain::folder_preview_validity::FolderPreviewValidity;
use rusqlite::params;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Borrowed preview pixels and their dimensions; writing never copies the input buffer.
#[derive(Clone, Copy)]
pub struct FolderPreviewPixels<'a> {
    pub rgba: &'a [u8],
    pub width: u32,
    pub height: u32,
}

fn preserves_larger_variant(
    db: &rusqlite::Connection,
    path: &Path,
    version: i64,
    bucket: u32,
    fingerprint: Option<&str>,
) -> bool {
    let existing: Option<(u32, Option<String>)> = db
        .query_row(
            "SELECT MAX(width, bucket_size), source_fingerprint FROM folder_preview_variants
         WHERE folder_path = ? AND format_version = ?",
            params![path.to_string_lossy(), version],
            |row| Ok((row.get::<_, i64>(0)? as u32, row.get(1)?)),
        )
        .ok();
    existing
        .is_some_and(|(capacity, stored)| capacity >= bucket && stored.as_deref() == fingerprint)
}

impl ThumbnailDiskCache {
    pub fn put_folder_preview_cache(
        &self,
        path: &Path,
        bucket: u32,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) {
        self.put_folder_preview_for_appearance(
            path,
            bucket,
            rgba,
            width,
            height,
            FolderAppearance::Original,
        );
    }

    pub fn put_folder_preview_for_appearance(
        &self,
        path: &Path,
        bucket: u32,
        rgba: &[u8],
        width: u32,
        height: u32,
        appearance: FolderAppearance,
    ) {
        let source = FolderPreviewSource::capture(path, None);
        self.put_folder_preview_inner(
            path,
            bucket,
            FolderPreviewPixels {
                rgba,
                width,
                height,
            },
            appearance,
            source.as_ref(),
            None,
        );
    }

    /// Commit only if the source and request are still current after acquiring the writer lock.
    pub fn put_folder_preview_checked(
        &self,
        path: &Path,
        bucket: u32,
        pixels: FolderPreviewPixels<'_>,
        appearance: FolderAppearance,
        source: &FolderPreviewSource,
        validity: &FolderPreviewValidity,
    ) {
        self.put_folder_preview_inner(
            path,
            bucket,
            pixels,
            appearance,
            Some(source),
            Some(validity),
        );
    }

    fn put_folder_preview_inner(
        &self,
        path: &Path,
        bucket: u32,
        pixels: FolderPreviewPixels<'_>,
        appearance: FolderAppearance,
        source: Option<&FolderPreviewSource>,
        validity: Option<&FolderPreviewValidity>,
    ) {
        let FolderPreviewPixels {
            rgba,
            width,
            height,
        } = pixels;
        let expected = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4));
        if expected != Some(rgba.len()) || width == 0 || height == 0 {
            return;
        }
        let version = appearance.cache_version(FOLDER_PREVIEW_CACHE_FORMAT_VERSION);
        let fingerprint = source.map(FolderPreviewSource::fingerprint);
        if validity.is_some_and(|token| !token.is_current())
            || source.is_some_and(|source| !source.is_current())
        {
            return;
        }

        {
            let db = self.writer.lock();
            if validity.is_some_and(|token| !token.is_current())
                || preserves_larger_variant(&db, path, version, bucket, fingerprint)
            {
                return;
            }
        }

        let data = webp::Encoder::from_rgba(rgba, width, height).encode_lossless();
        // Invalidation can occur while encoding; never resurrect its old cache entry.
        if validity.is_some_and(|token| !token.is_current())
            || source.is_some_and(|source| !source.is_current())
        {
            return;
        }
        // Metadata I/O and compression must not hold the shared SQLite writer lock.
        let db = self.writer.lock();
        if validity.is_some_and(|token| !token.is_current())
            || preserves_larger_variant(&db, path, version, bucket, fingerprint)
        {
            return;
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let _ = db.execute(
            "INSERT OR REPLACE INTO folder_preview_variants
             (folder_path, data, width, height, bucket_size, format_version, created_at, source_fingerprint)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            params![path.to_string_lossy(), data.to_vec(), width, height, bucket, version, now, fingerprint],
        );
    }
}
