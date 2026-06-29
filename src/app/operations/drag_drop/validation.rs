use crate::app::state::ImageViewerApp;
use crate::domain::file_entry::is_path_inside_existing_archive_file;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DragDropOperation {
    Copy,
    Move,
}

/// Returns true when any dragged path is a virtual entry inside an archive
/// (e.g. `C:\foo.zip\bar\item.txt`). Such items cannot be moved by the
/// Windows Shell, so the drag must be treated as a copy regardless of
/// modifiers, volume or dual-panel context.
pub(super) fn drag_payload_inside_archive(paths: &[PathBuf]) -> bool {
    paths
        .iter()
        .any(|p| is_path_inside_existing_archive_file(p))
}

impl ImageViewerApp {
    pub(super) fn collect_drag_payload(&self, item_idx: usize) -> Vec<PathBuf> {
        let Some(item) = self.items.get(item_idx) else {
            return Vec::new();
        };

        if self.multi_selection.contains(&item.path) && !self.multi_selection.is_empty() {
            let mut paths: Vec<PathBuf> = self.multi_selection.iter().cloned().collect();
            paths.retain(|path| self.items.iter().any(|it| it.path == *path));
            if paths.is_empty() {
                paths.push(item.path.clone());
            }
            return paths;
        }

        vec![item.path.clone()]
    }

    pub(crate) fn is_valid_drop_target(&self, target: &Path) -> bool {
        is_valid_drop_target_for_paths(&self.drag_payload_paths, target)
    }

    pub(super) fn resolve_drag_operation(
        &self,
        dest_folder: &Path,
        ctrl_pressed: bool,
        shift_pressed: bool,
    ) -> DragDropOperation {
        resolve_drag_operation_for_paths(
            &self.drag_payload_paths,
            dest_folder,
            ctrl_pressed,
            shift_pressed,
        )
    }
}

fn resolve_drag_operation_for_paths(
    paths: &[PathBuf],
    _dest_folder: &Path,
    _ctrl_pressed: bool,
    _shift_pressed: bool,
) -> DragDropOperation {
    // Items inside archives (virtual paths like `C:\foo.zip\bar.txt`)
    // cannot be moved by the Shell. Every other internal drag is a move.
    if drag_payload_inside_archive(paths) {
        DragDropOperation::Copy
    } else {
        DragDropOperation::Move
    }
}

pub(super) fn is_valid_drop_target_for_paths(paths: &[PathBuf], target: &Path) -> bool {
    if paths.is_empty() {
        return false;
    }

    if ImageViewerApp::path_is_archive_namespace(target) {
        return false;
    }

    let target_norm = normalize_path_for_compare(target);

    for source in paths {
        let source_norm = normalize_path_for_compare(source);

        if source_norm == target_norm {
            return false;
        }

        let source_prefix = format!("{source_norm}\\");
        if target_norm.starts_with(&source_prefix) {
            return false;
        }
    }

    !paths.iter().all(|source| {
        source
            .parent()
            .is_some_and(|p| normalize_path_for_compare(p) == target_norm)
    })
}

pub(super) fn should_confirm_drag_move(operation: DragDropOperation) -> bool {
    matches!(operation, DragDropOperation::Move)
}

pub(super) fn should_block_file_panel_input_for_pending_confirmation(
    has_pending_confirmation: bool,
) -> bool {
    has_pending_confirmation
}

pub(super) fn normalize_path_for_compare(path: &Path) -> String {
    let lower = path.to_string_lossy().to_ascii_lowercase();
    if let Some(stripped) = lower.strip_prefix(r"\\?\") {
        stripped.to_string()
    } else {
        lower
    }
}

#[cfg(test)]
mod tests {
    use super::{
        drag_payload_inside_archive, is_valid_drop_target_for_paths,
        resolve_drag_operation_for_paths, should_block_file_panel_input_for_pending_confirmation,
        should_confirm_drag_move, DragDropOperation,
    };
    use std::path::{Path, PathBuf};

    #[test]
    fn drag_payload_inside_archive_detects_virtual_paths() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let archive = dir.path().join("archive.zip");
        std::fs::write(&archive, b"zip placeholder").expect("create archive file");

        let paths = vec![archive.join("inner").join("file.txt")];
        assert!(drag_payload_inside_archive(&paths));
    }

    #[test]
    fn drag_payload_inside_archive_ignores_plain_files() {
        let paths = vec![
            PathBuf::from(r"C:\Windows\notepad.exe"),
            PathBuf::from(r"E:\movies\trailer.mp4"),
        ];
        assert!(!drag_payload_inside_archive(&paths));
    }

    #[test]
    fn drag_payload_inside_archive_does_not_treat_archive_root_as_inside() {
        // `C:\file.zip` is the archive file itself, NOT a virtual entry inside it.
        let dir = tempfile::tempdir().expect("create temp dir");
        let archive = dir.path().join("file.zip");
        std::fs::write(&archive, b"zip placeholder").expect("create archive file");

        let paths = vec![archive];
        assert!(!drag_payload_inside_archive(&paths));
    }

    #[test]
    fn drag_payload_inside_archive_ignores_real_directory_named_like_archive() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let archive_named_dir = dir.path().join("project.zip");
        std::fs::create_dir(&archive_named_dir).expect("create archive-named directory");

        let paths = vec![archive_named_dir.join("file.txt")];
        assert!(!drag_payload_inside_archive(&paths));
    }

    #[test]
    fn drag_operation_moves_regular_items() {
        let paths = vec![PathBuf::from(r"G:\Source\file.txt")];

        assert_eq!(
            resolve_drag_operation_for_paths(&paths, Path::new(r"G:\Dest"), false, false),
            DragDropOperation::Move
        );
    }

    #[test]
    fn drag_operation_moves_regular_items_across_volumes() {
        let paths = vec![PathBuf::from(r"G:\Source\file.txt")];

        assert_eq!(
            resolve_drag_operation_for_paths(&paths, Path::new(r"E:\Dest"), false, false),
            DragDropOperation::Move
        );
    }

    #[test]
    fn drag_operation_moves_regular_items_even_with_ctrl() {
        let paths = vec![PathBuf::from(r"G:\Source\file.txt")];

        assert_eq!(
            resolve_drag_operation_for_paths(&paths, Path::new(r"E:\Dest"), true, false),
            DragDropOperation::Move
        );
    }

    #[test]
    fn drag_operation_copies_archive_entries_even_with_shift() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let archive = dir.path().join("archive.zip");
        std::fs::write(&archive, b"zip placeholder").expect("create archive file");
        let paths = vec![archive.join("inner").join("file.txt")];

        assert_eq!(
            resolve_drag_operation_for_paths(&paths, dir.path(), false, true),
            DragDropOperation::Copy
        );
    }

    #[test]
    fn drop_target_rejects_archive_root_file() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let archive = dir.path().join("archive.zip");
        std::fs::write(&archive, b"zip placeholder").expect("create archive file");
        let paths = vec![dir.path().join("file.txt")];

        assert!(!is_valid_drop_target_for_paths(&paths, &archive));
    }

    #[test]
    fn drop_target_rejects_path_inside_archive() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let archive = dir.path().join("archive.zip");
        std::fs::write(&archive, b"zip placeholder").expect("create archive file");
        let paths = vec![dir.path().join("file.txt")];

        assert!(!is_valid_drop_target_for_paths(
            &paths,
            &archive.join("inner")
        ));
    }

    #[test]
    fn drop_target_allows_real_directory_named_like_archive() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let archive_named_dir = dir.path().join("project.zip");
        std::fs::create_dir(&archive_named_dir).expect("create archive-named dir");
        let paths = vec![dir.path().join("file.txt")];

        assert!(is_valid_drop_target_for_paths(&paths, &archive_named_dir));
    }

    #[test]
    fn pending_confirmation_blocks_file_panel_input() {
        assert!(should_block_file_panel_input_for_pending_confirmation(true));
    }

    #[test]
    fn missing_confirmation_does_not_block_file_panel_input() {
        assert!(!should_block_file_panel_input_for_pending_confirmation(
            false
        ));
    }

    #[test]
    fn any_move_requires_confirmation() {
        assert!(should_confirm_drag_move(DragDropOperation::Move));
    }

    #[test]
    fn copy_does_not_require_confirmation() {
        assert!(!should_confirm_drag_move(DragDropOperation::Copy));
    }

    #[test]
    fn drop_target_rejects_source_itself() {
        let paths = vec![PathBuf::from(r"C:\Source\Folder")];

        assert!(!is_valid_drop_target_for_paths(
            &paths,
            Path::new(r"C:\Source\Folder")
        ));
    }

    #[test]
    fn drop_target_rejects_source_descendant() {
        let paths = vec![PathBuf::from(r"C:\Source\Folder")];

        assert!(!is_valid_drop_target_for_paths(
            &paths,
            Path::new(r"C:\Source\Folder\Child")
        ));
    }

    #[test]
    fn drop_target_rejects_when_all_sources_are_already_children() {
        let paths = vec![
            PathBuf::from(r"C:\Source\a.txt"),
            PathBuf::from(r"C:\Source\b.txt"),
        ];

        assert!(!is_valid_drop_target_for_paths(
            &paths,
            Path::new(r"C:\Source")
        ));
    }

    #[test]
    fn drop_target_accepts_different_folder() {
        let paths = vec![PathBuf::from(r"C:\Source\a.txt")];

        assert!(is_valid_drop_target_for_paths(
            &paths,
            Path::new(r"C:\Dest")
        ));
    }
}
