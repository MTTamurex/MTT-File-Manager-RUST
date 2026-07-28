//! UI Rendering bridges - simplified coordination module
//!
//! This module provides bridge implementations between App state and UI views,
//! delegating actual rendering to specialized view modules.

pub mod column_list_bridge;
pub mod grid_bridge;
pub mod item_slot_bridge;
pub mod list_bridge;
pub mod list_folder_previews;
pub mod miller_bridge;
mod miller_interactions;
mod miller_selection;

fn ordered_context_menu_paths(
    primary: &std::path::Path,
    selection: &crate::ui::cache::FxHashSet<std::path::PathBuf>,
) -> Vec<std::path::PathBuf> {
    let mut paths = vec![primary.to_path_buf()];
    paths.extend(
        selection
            .iter()
            .filter(|path| path.as_path() != primary)
            .cloned(),
    );
    paths
}

// Re-export commonly used types
pub use grid_bridge::*;
pub use list_bridge::*;

#[cfg(test)]
mod tests {
    use super::ordered_context_menu_paths;
    use crate::ui::cache::FxHashSet;
    use std::path::{Path, PathBuf};

    #[test]
    fn context_menu_paths_keep_clicked_item_first() {
        let primary = PathBuf::from(r"C:\folder\clicked");
        let mut selection = FxHashSet::default();
        selection.insert(PathBuf::from(r"C:\folder\other"));
        selection.insert(primary.clone());

        let paths = ordered_context_menu_paths(&primary, &selection);
        assert_eq!(paths.first().map(PathBuf::as_path), Some(primary.as_path()));
        assert_eq!(paths.len(), 2);
        assert!(paths.contains(&Path::new(r"C:\folder\other").to_path_buf()));
    }
}
