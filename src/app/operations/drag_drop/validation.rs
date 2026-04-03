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

    pub(super) fn is_valid_drop_target(&self, target: &Path) -> bool {
        if self.drag_payload_paths.is_empty() {
            return false;
        }

        let target_norm = normalize_path_for_compare(target);

        for source in &self.drag_payload_paths {
            let source_norm = normalize_path_for_compare(source);

            // Can't drop onto itself.
            if source_norm == target_norm {
                return false;
            }

            // Can't drop a folder into itself/descendant.
            let source_prefix = format!("{source_norm}\\");
            if target_norm.starts_with(&source_prefix) {
                return false;
            }
        }

        // No-op: reject if ALL sources are already direct children of the target folder.
        let all_already_in_target = self.drag_payload_paths.iter().all(|source| {
            source
                .parent()
                .is_some_and(|p| normalize_path_for_compare(p) == target_norm)
        });
        if all_already_in_target {
            return false;
        }

        true
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
