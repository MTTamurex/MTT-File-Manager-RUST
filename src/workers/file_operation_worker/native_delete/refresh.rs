//! Bounded batches report real removals, including children of a surviving root.
use super::FileOperationResult;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

pub(super) struct RemovedItems {
    sender: Sender<FileOperationResult>,
    paths: Vec<PathBuf>,
}

impl RemovedItems {
    pub(super) fn new(sender: Sender<FileOperationResult>) -> Self {
        Self {
            sender,
            paths: Vec::with_capacity(128),
        }
    }

    pub(super) fn record(&mut self, path: &Path) {
        self.paths.push(path.to_path_buf());
        if self.paths.len() >= 128 {
            self.flush();
        }
    }

    pub(super) fn flush(&mut self) {
        if self.paths.is_empty() {
            return;
        }
        let deleted_paths = std::mem::replace(&mut self.paths, Vec::with_capacity(128));
        let parents: HashSet<_> = deleted_paths
            .iter()
            .filter_map(|path| path.parent().map(Path::to_path_buf))
            .collect();
        let _ = self.sender.send(FileOperationResult::DeleteCompleted {
            parent_folders: parents.into_iter().collect(),
            deleted_paths,
        });
    }
}

impl Drop for RemovedItems {
    fn drop(&mut self) {
        self.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn removal_batches_are_bounded_and_flush_on_drop() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let mut removed = RemovedItems::new(sender);
        for index in 0..129 {
            removed.record(&PathBuf::from(format!("source/file-{index}.txt")));
            assert!(removed.paths.len() < 128);
        }
        let FileOperationResult::DeleteCompleted {
            deleted_paths,
            parent_folders,
        } = receiver.try_recv().unwrap()
        else {
            panic!("Expected deletion result");
        };
        assert_eq!(deleted_paths.len(), 128);
        assert_eq!(parent_folders, vec![PathBuf::from("source")]);
        assert!(receiver.try_recv().is_err());
        drop(removed);
        let FileOperationResult::DeleteCompleted { deleted_paths, .. } =
            receiver.try_recv().unwrap()
        else {
            panic!("Expected final partial batch");
        };
        assert_eq!(deleted_paths, vec![PathBuf::from("source/file-128.txt")]);
    }
}
