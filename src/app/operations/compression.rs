//! Context-menu "Compress" dispatch: archive naming, collision avoidance,
//! and worker send.

use std::path::{Path, PathBuf};

use crate::app::state::ImageViewerApp;
use crate::infrastructure::archive_create::CompressionFormat;
use crate::workers::archive_compression_worker::ArchiveCompressionRequest;

/// Default archive name for a selection (Explorer-style): the file stem of
/// the first selected item.
fn default_archive_stem(targets: &[PathBuf]) -> String {
    targets
        .first()
        .and_then(|path| path.file_stem())
        .map(|stem| stem.to_string_lossy().to_string())
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| "archive".to_string())
}

/// Returns `folder/base.ext`, appending " (n)" before the extension while the
/// candidate already exists. Existing archives are never overwritten.
pub(crate) fn unique_archive_path(folder: &Path, base: &str, extension: &str) -> PathBuf {
    let mut candidate = folder.join(format!("{base}.{extension}"));
    let mut counter = 1;
    while candidate.exists() && counter <= 1000 {
        counter += 1;
        candidate = folder.join(format!("{base} ({counter}).{extension}"));
    }
    candidate
}

impl ImageViewerApp {
    /// Starts compressing `targets` into a new archive placed in the folder
    /// that contains them. Fire-and-forget: progress arrives via the shared
    /// compression progress toast, completion via `CompressionCompleted`.
    pub fn begin_compression(&mut self, targets: &[PathBuf], format: CompressionFormat) {
        let Some(first) = targets.first() else {
            return;
        };
        let Some(dest_folder) = first.parent() else {
            return;
        };

        let stem = default_archive_stem(targets);
        let dest = unique_archive_path(dest_folder, &stem, format.extension());
        self.file_operation_state.file_ops_in_progress += 1;
        let request = ArchiveCompressionRequest::Compress {
            sources: targets.to_vec(),
            dest,
            format,
        };
        if self
            .file_operation_state
            .compression_sender
            .send(request)
            .is_err()
        {
            self.file_operation_state.file_ops_in_progress = self
                .file_operation_state
                .file_ops_in_progress
                .saturating_sub(1);
            log::warn!("[Compress] worker channel closed on compress request");
            self.notifications
                .warning(rust_i18n::t!("operations.compress_dispatch_failed"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{default_archive_stem, unique_archive_path};

    #[test]
    fn archive_stem_uses_first_target_file_stem() {
        let targets = vec![
            std::path::PathBuf::from(r"C:\docs\Report.final.txt"),
            std::path::PathBuf::from(r"C:\docs\other.bin"),
        ];
        assert_eq!(default_archive_stem(&targets), "Report.final");
    }

    #[test]
    fn archive_stem_falls_back_for_rootless_paths() {
        assert_eq!(
            default_archive_stem(&[std::path::PathBuf::from(r"\\")]),
            "archive"
        );
        assert_eq!(default_archive_stem(&[]), "archive");
    }

    #[test]
    fn unique_path_appends_counter_on_collision() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("docs.zip"), b"").unwrap();

        let first = unique_archive_path(temp.path(), "docs", "zip");
        assert_eq!(first, temp.path().join("docs (2).zip"));

        std::fs::write(temp.path().join("docs (2).zip"), b"").unwrap();
        let second = unique_archive_path(temp.path(), "docs", "zip");
        assert_eq!(second, temp.path().join("docs (3).zip"));
    }

    #[test]
    fn unique_path_returns_plain_name_when_free() {
        let temp = tempfile::tempdir().unwrap();
        let path = unique_archive_path(temp.path(), "free", "7z");
        assert_eq!(path, temp.path().join("free.7z"));
    }
}
