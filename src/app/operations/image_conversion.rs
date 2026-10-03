use std::path::{Path, PathBuf};

use crate::app::state::ImageViewerApp;
use crate::image_viewer::loader::ExportImageFormat;
use crate::workers::image_conversion_worker::ImageConversionRequest;

impl ImageViewerApp {
    pub fn begin_image_conversion(&mut self, targets: &[PathBuf], format: ExportImageFormat) {
        let Some(source) = targets.first() else {
            return;
        };
        let Some(dest_folder) = source.parent() else {
            return;
        };
        let stem = source
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_string())
            .filter(|stem| !stem.is_empty())
            .unwrap_or_else(|| "image".to_string());
        let dest = unique_conversion_path(dest_folder, &stem, format.extension());

        self.file_operation_state.file_ops_in_progress += 1;
        if self
            .file_operation_state
            .conversion_sender
            .send(ImageConversionRequest::Convert {
                source: source.clone(),
                dest,
                format,
            })
            .is_err()
        {
            self.file_operation_state.file_ops_in_progress = self
                .file_operation_state
                .file_ops_in_progress
                .saturating_sub(1);
            log::warn!("[ImageConversion] worker channel closed on conversion request");
            self.notifications
                .warning(rust_i18n::t!("operations.convert_dispatch_failed"));
        }
    }
}

fn unique_conversion_path(folder: &Path, base: &str, extension: &str) -> PathBuf {
    crate::app::operations::compression::unique_archive_path(folder, base, extension)
}

#[cfg(test)]
mod tests {
    use super::unique_conversion_path;

    #[test]
    fn conversion_path_adds_counter_on_collision() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("image.webp"), b"").unwrap();

        assert_eq!(
            unique_conversion_path(temp.path(), "image", "webp"),
            temp.path().join("image (2).webp")
        );
    }

    #[test]
    fn conversion_path_uses_plain_name_when_available() {
        let temp = tempfile::tempdir().unwrap();

        assert_eq!(
            unique_conversion_path(temp.path(), "image", "pdf"),
            temp.path().join("image.pdf")
        );
    }
}
