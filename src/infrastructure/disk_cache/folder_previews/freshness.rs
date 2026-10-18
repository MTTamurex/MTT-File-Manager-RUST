use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// Exact source metadata captured before cache lookup/composition, not its completion time.
#[derive(Clone, Debug)]
pub struct FolderPreviewSource {
    folder: PathBuf,
    cover: Option<PathBuf>,
    fingerprint: String,
}

impl FolderPreviewSource {
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
