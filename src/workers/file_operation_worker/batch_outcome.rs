//! Conservative filesystem evidence for partially completed Shell batches.
use std::path::{Path, PathBuf};

pub(super) fn existing_paths(paths: &[PathBuf]) -> Vec<PathBuf> {
    paths
        .iter()
        .filter(|path| std::fs::symlink_metadata(path).is_ok())
        .cloned()
        .collect()
}

pub(super) fn definitely_absent(path: &Path) -> bool {
    matches!(std::fs::symlink_metadata(path), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
}

pub(super) fn completed_sources(existing: Vec<PathBuf>) -> Vec<PathBuf> {
    existing
        .into_iter()
        .filter(|path| definitely_absent(path))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_batch_reports_only_sources_that_disappeared() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("first.txt");
        let second = directory.path().join("second.txt");
        let missing = directory.path().join("missing.txt");
        std::fs::write(&first, "test").unwrap();
        std::fs::write(&second, "test").unwrap();
        let existing = existing_paths(&[first.clone(), second.clone(), missing]);
        std::fs::rename(&first, directory.path().join("moved.txt")).unwrap();
        assert_eq!(completed_sources(existing), [first]);
        assert!(!definitely_absent(&second));
    }
}
