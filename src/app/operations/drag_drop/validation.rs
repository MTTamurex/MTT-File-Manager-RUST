use crate::app::state::ImageViewerApp;
use crate::domain::file_entry::is_path_inside_existing_archive_file;
use crate::domain::network_path::is_network_location_root;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DragDropOperation {
    Copy,
    Move,
    Link,
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

        if is_network_location_root(&item.path)
            || self
                .multi_selection
                .iter()
                .any(|path| is_network_location_root(path))
        {
            return Vec::new();
        }

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
        let (ctrl, shift) = self.ui_ctx.input(|i| (i.modifiers.ctrl, i.modifiers.shift));
        is_valid_drop_target_for_operation(
            &self.drag_payload_paths,
            &self.resolve_network_location_path(target),
            self.resolve_drag_operation(target, ctrl, shift),
        )
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
            self.ui_ctx.input(|i| i.modifiers.alt),
        )
    }
}

fn resolve_drag_operation_for_paths(
    paths: &[PathBuf],
    _dest_folder: &Path,
    ctrl_pressed: bool,
    _shift_pressed: bool,
    alt_pressed: bool,
) -> DragDropOperation {
    // Items inside archives (virtual paths like `C:\foo.zip\bar.txt`)
    // cannot be moved or linked as filesystem items. Preserve extraction by copy.
    if drag_payload_inside_archive(paths) {
        DragDropOperation::Copy
    } else if alt_pressed {
        DragDropOperation::Link
    } else if ctrl_pressed {
        DragDropOperation::Copy
    } else {
        DragDropOperation::Move
    }
}

pub(super) fn is_valid_drop_target_for_paths(paths: &[PathBuf], target: &Path) -> bool {
    is_valid_drop_target_for_operation(paths, target, DragDropOperation::Move)
}

fn is_valid_drop_target_for_operation(
    paths: &[PathBuf],
    target: &Path,
    operation: DragDropOperation,
) -> bool {
    if paths.is_empty() || paths.iter().any(|path| is_network_location_root(path)) {
        return false;
    }

    if !super::super::file_operation_policy::path_accepts_file_children(target) {
        return false;
    }

    let target_norm = normalize_path_for_compare(target);

    for source in paths {
        let source_norm = normalize_path_for_compare(source);

        if operation != DragDropOperation::Link && source_norm == target_norm {
            return false;
        }

        let source_prefix = format!("{source_norm}\\");
        if operation != DragDropOperation::Link && target_norm.starts_with(&source_prefix) {
            return false;
        }
    }

    operation != DragDropOperation::Move
        || !paths.iter().any(|source| {
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
    let normalized = if let Some(stripped) = lower.strip_prefix(r"\\?\unc\") {
        format!(r"\\{stripped}")
    } else if let Some(stripped) = lower.strip_prefix(r"\\?\") {
        stripped.to_string()
    } else {
        lower
    };
    if normalized.len() > 3 {
        normalized.trim_end_matches(['\\', '/']).to_string()
    } else {
        normalized
    }
}

#[cfg(test)]
mod modifier_tests;

#[cfg(test)]
mod tests {
    use super::{
        drag_payload_inside_archive, is_valid_drop_target_for_paths,
        resolve_drag_operation_for_paths, should_block_file_panel_input_for_pending_confirmation,
        should_confirm_drag_move, DragDropOperation,
    };
    use std::path::{Path, PathBuf};

    #[test]
    fn network_drops_distinguish_servers_shares_and_share_contents() {
        let sources = vec![PathBuf::from(r"C:\Folder\file.txt")];
        assert!(!is_valid_drop_target_for_paths(
            &sources,
            Path::new(r"\\server")
        ));
        assert!(is_valid_drop_target_for_paths(
            &sources,
            Path::new(r"\\server\share")
        ));
        assert!(!is_valid_drop_target_for_paths(
            &[PathBuf::from(r"\\server\share")],
            Path::new(r"C:\Folder")
        ));
        assert!(is_valid_drop_target_for_paths(
            &[PathBuf::from(r"\\server\share\file.txt")],
            Path::new(r"C:\Folder")
        ));
    }

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
            resolve_drag_operation_for_paths(&paths, Path::new(r"G:\Dest"), false, false, false),
            DragDropOperation::Move
        );
    }

    #[test]
    fn drag_operation_moves_regular_items_across_volumes() {
        let paths = vec![PathBuf::from(r"G:\Source\file.txt")];

        assert_eq!(
            resolve_drag_operation_for_paths(&paths, Path::new(r"E:\Dest"), false, false, false),
            DragDropOperation::Move
        );
    }

    #[test]
    fn drag_operation_copies_regular_items_with_ctrl() {
        let paths = vec![PathBuf::from(r"G:\Source\file.txt")];

        assert_eq!(
            resolve_drag_operation_for_paths(&paths, Path::new(r"E:\Dest"), true, false, false),
            DragDropOperation::Copy
        );
    }

    #[test]
    fn drag_operation_copies_archive_entries_even_with_shift() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let archive = dir.path().join("archive.zip");
        std::fs::write(&archive, b"zip placeholder").expect("create archive file");
        let paths = vec![archive.join("inner").join("file.txt")];

        assert_eq!(
            resolve_drag_operation_for_paths(&paths, dir.path(), false, true, false),
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
    fn drop_target_rejects_when_any_source_is_already_a_child() {
        let paths = vec![
            PathBuf::from(r"C:\Source\a.txt"),
            PathBuf::from(r"C:\Other\b.txt"),
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

    #[test]
    fn drop_target_matches_extended_and_regular_unc_paths() {
        let paths = vec![PathBuf::from(r"\\?\UNC\server\share\a.txt")];

        assert!(!is_valid_drop_target_for_paths(
            &paths,
            Path::new(r"\\server\share")
        ));
    }
}
