use super::{sanitize_operation_path, sanitize_operation_paths, FileOperationResult};
use std::path::PathBuf;
use std::sync::mpsc::Sender;

/// Create every selected shortcut on the STA worker, retaining successful results
/// when another item fails so that the destination view is still refreshed.
pub(super) fn create_shortcuts(
    paths: Vec<PathBuf>,
    dest_folder: PathBuf,
    sender: &Sender<FileOperationResult>,
) {
    let result = sanitize_operation_paths(&paths).and_then(|paths| {
        let destination = sanitize_operation_path(&dest_folder)?;
        let mut created = Vec::new();
        let mut failure = None;
        for path in paths {
            match crate::application::file_operations::create_shortcut(
                &path,
                &destination.to_string_lossy(),
            ) {
                Ok(path) => created.push(path),
                Err(error) => failure = Some(error),
            }
        }
        let _ = sender.send(FileOperationResult::CopyCompleted {
            dest_folder: destination,
            copied_dests: created,
        });
        failure.map_or(Ok(()), Err)
    });
    if let Err(message) = result {
        let _ = sender.send(FileOperationResult::OperationFailed { message });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::windows::shortcuts::resolve_shortcut_target;

    #[test]
    fn multi_shortcuts_use_destination_unique_names_and_correct_targets() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("Source");
        let destination = temp.path().join("Destination");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&destination).unwrap();
        let file = source.join("file.txt");
        let folder = source.join("Folder");
        std::fs::write(&file, b"original").unwrap();
        std::fs::create_dir(&folder).unwrap();
        let paths = vec![file.clone(), folder.clone()];
        let (sender, receiver) = std::sync::mpsc::channel();
        // Repeated operations must not overwrite previous shortcuts.
        for _ in 0..2 {
            create_shortcuts(paths.clone(), destination.clone(), &sender);
            let FileOperationResult::CopyCompleted {
                dest_folder,
                copied_dests,
            } = receiver.recv().unwrap()
            else {
                panic!("expected shortcut completion");
            };
            assert_eq!(
                std::fs::canonicalize(dest_folder).unwrap(),
                std::fs::canonicalize(&destination).unwrap()
            );
            assert_eq!(copied_dests.len(), paths.len());
            for (shortcut, target) in copied_dests.iter().zip(&paths) {
                assert_eq!(shortcut.extension().unwrap(), "lnk");
                assert_eq!(
                    std::fs::canonicalize(shortcut.parent().unwrap()).unwrap(),
                    std::fs::canonicalize(&destination).unwrap()
                );
                let resolved = resolve_shortcut_target(shortcut).expect("valid shortcut");
                assert_eq!(
                    std::fs::canonicalize(resolved).unwrap(),
                    std::fs::canonicalize(target).unwrap()
                );
            }
        }
        assert_eq!(std::fs::read_dir(&destination).unwrap().count(), 4);
        assert_eq!(std::fs::read(&file).unwrap(), b"original");
        assert_eq!(std::fs::read_dir(&source).unwrap().count(), 2);
        // Also exercise dropping the complete selection into its own directory.
        create_shortcuts(paths, source.clone(), &sender);
        let FileOperationResult::CopyCompleted { copied_dests, .. } = receiver.recv().unwrap()
        else {
            panic!("expected same-folder shortcut completion");
        };
        assert_eq!(copied_dests.len(), 2);
        assert_eq!(std::fs::read_dir(&source).unwrap().count(), 4);
    }

    #[test]
    fn selection_from_different_folders_preserves_equal_named_targets() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("First");
        let second = temp.path().join("Second");
        let destination = temp.path().join("Destination");
        for directory in [&first, &second, &destination] {
            std::fs::create_dir(directory).unwrap();
        }
        let paths = vec![first.join("file.txt"), second.join("file.txt")];
        for (index, path) in paths.iter().enumerate() {
            std::fs::write(path, index.to_string()).unwrap();
        }
        let (sender, receiver) = std::sync::mpsc::channel();
        create_shortcuts(paths.clone(), destination.clone(), &sender);
        let FileOperationResult::CopyCompleted { copied_dests, .. } = receiver.recv().unwrap()
        else {
            panic!("expected shortcut completion");
        };
        assert_eq!(copied_dests.len(), 2);
        assert_ne!(copied_dests[0], copied_dests[1]);
        for (shortcut, target) in copied_dests.iter().zip(paths) {
            let resolved = resolve_shortcut_target(shortcut).unwrap();
            assert_eq!(
                std::fs::canonicalize(resolved).unwrap(),
                std::fs::canonicalize(target).unwrap()
            );
        }
    }
}
