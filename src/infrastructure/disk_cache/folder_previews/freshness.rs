use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// Exact source identity attached to preview pixels. Fingerprints read from
/// SQLite remain snapshots until background source validation completes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderPreviewSource {
    folder: PathBuf,
    cover: Option<PathBuf>,
    fingerprint: String,
}

impl FolderPreviewSource {
    /// Reconstructs the identity stored with cached pixels, without filesystem I/O.
    /// It is only a snapshot until `capture`/`is_current` validates it.
    pub(crate) fn from_cached_fingerprint(
        folder: &Path,
        cover: Option<&Path>,
        fingerprint: String,
    ) -> Self {
        Self {
            folder: folder.to_path_buf(),
            cover: cover.map(Path::to_path_buf),
            fingerprint,
        }
    }

    pub(crate) fn matches_paths(&self, folder: &Path, cover: Option<&Path>) -> bool {
        self.folder == folder && self.cover.as_deref() == cover
    }

    pub fn capture(folder: &Path, cover: Option<&Path>) -> Option<Self> {
        let mut hash = blake3::Hasher::new();
        for path in std::iter::once(folder).chain(cover) {
            let metadata = std::fs::metadata(path).ok()?;
            hash.update(path.to_string_lossy().as_bytes());
            hash.update(&[0]);
            hash.update(&metadata.len().to_le_bytes());
            hash.update(
                &metadata
                    .modified()
                    .ok()?
                    .duration_since(UNIX_EPOCH)
                    .ok()?
                    .as_nanos()
                    .to_le_bytes(),
            );
            if let Ok(created) = metadata.created().and_then(|time| {
                time.duration_since(UNIX_EPOCH)
                    .map_err(std::io::Error::other)
            }) {
                hash.update(&created.as_nanos().to_le_bytes());
            }
        }
        Some(Self {
            folder: folder.to_path_buf(),
            cover: cover.map(Path::to_path_buf),
            fingerprint: hash.finalize().to_hex().to_string(),
        })
    }

    pub(super) fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    pub fn is_current(&self) -> bool {
        Self::capture(&self.folder, self.cover.as_deref())
            .is_some_and(|source| source.fingerprint == self.fingerprint)
    }
}

#[cfg(test)]
mod tests;
