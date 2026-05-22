use crate::app::state::ImageViewerApp;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DragDropOperation {
    Copy,
    Move,
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
        if ctrl_pressed {
            return DragDropOperation::Copy;
        }
        if shift_pressed {
            return DragDropOperation::Move;
        }

        let target_volume = volume_key(dest_folder);
        let same_volume_for_all = self.drag_payload_paths.iter().all(|source| {
            let base = source.parent().unwrap_or(source.as_path());
            volume_key(base) == target_volume
        });

        if same_volume_for_all {
            DragDropOperation::Move
        } else {
            DragDropOperation::Copy
        }
    }
}

pub(super) fn is_valid_drop_target_for_paths(paths: &[PathBuf], target: &Path) -> bool {
    if paths.is_empty() {
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

pub(super) fn should_confirm_cross_panel_move(
    source_cross_panel_context: bool,
    target_cross_panel_context: bool,
    operation: DragDropOperation,
) -> bool {
    source_cross_panel_context != target_cross_panel_context
        && matches!(operation, DragDropOperation::Move)
}

pub(super) fn normalize_path_for_compare(path: &Path) -> String {
    let lower = path.to_string_lossy().to_ascii_lowercase();
    if let Some(stripped) = lower.strip_prefix(r"\\?\") {
        stripped.to_string()
    } else {
        lower
    }
}

fn volume_key(path: &Path) -> Option<String> {
    path.components().find_map(|comp| match comp {
        Component::Prefix(prefix) => {
            Some(prefix.as_os_str().to_string_lossy().to_ascii_uppercase())
        }
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        is_valid_drop_target_for_paths, should_confirm_cross_panel_move, DragDropOperation,
    };
    use std::path::{Path, PathBuf};

    #[test]
    fn active_to_inactive_move_requires_confirmation() {
        assert!(should_confirm_cross_panel_move(
            false,
            true,
            DragDropOperation::Move
        ));
    }

    #[test]
    fn inactive_to_active_move_requires_confirmation() {
        assert!(should_confirm_cross_panel_move(
            true,
            false,
            DragDropOperation::Move
        ));
    }

    #[test]
    fn active_to_inactive_copy_does_not_require_confirmation() {
        assert!(!should_confirm_cross_panel_move(
            false,
            true,
            DragDropOperation::Copy
        ));
    }

    #[test]
    fn active_to_active_move_does_not_require_confirmation() {
        assert!(!should_confirm_cross_panel_move(
            false,
            false,
            DragDropOperation::Move
        ));
    }

    #[test]
    fn inactive_to_inactive_move_does_not_require_confirmation() {
        assert!(!should_confirm_cross_panel_move(
            true,
            true,
            DragDropOperation::Move
        ));
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
