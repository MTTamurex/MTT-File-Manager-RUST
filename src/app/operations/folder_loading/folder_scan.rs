use crate::app::state::ImageViewerApp;
use std::collections::HashSet;
use std::path::PathBuf;

impl ImageViewerApp {
    pub fn request_folder_scan(&mut self, folder_path: PathBuf) {
        self.request_folder_scans_batch(vec![folder_path]);
    }

    /// Even a local attribute read or a SQLite cache hit can block on a busy
    /// disk. Resolve and validate covers entirely on the existing cover worker;
    /// covers already carried by the listing remain visible in the meantime.
    pub fn request_folder_scans_batch(&mut self, folder_paths: Vec<PathBuf>) {
        let _stage = crate::infrastructure::diagnostic_logger::navigation_trace::Stage::new(
            "folder_covers_submit",
        );
        for path in unique_folder_paths(folder_paths) {
            if self.cover_worker_sender.send(path.clone()).is_err() {
                // Allow the renderer to request it again rather than marking an
                // undelivered request as permanently scanned.
                self.scanned_folders.pop(&path);
            }
        }
    }
}

fn unique_folder_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::with_capacity(paths.len());
    paths
        .into_iter()
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn batch_deduplicates_without_reordering() {
        let paths = ["folder-a", "folder-b", "folder-a"].map(PathBuf::from);
        assert_eq!(unique_folder_paths(paths.to_vec()), paths[..2]);
    }
}
