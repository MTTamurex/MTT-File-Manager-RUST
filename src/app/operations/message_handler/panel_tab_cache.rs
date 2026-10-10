use crate::app::dual_panel::PanelSnapshot;
use crate::app::state::ImageViewerApp;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;

fn panel_path_matches_folder(panel_path: &str, normalized_folder: &str) -> bool {
    let panel_path = normalize_panel_path(Path::new(panel_path));
    let normalized_folder = normalize_panel_path(Path::new(normalized_folder));
    panel_path == normalized_folder
}

fn normalize_panel_path(path: &Path) -> String {
    let mut normalized = ImageViewerApp::normalize_for_match(path);
    if Path::new(&normalized).is_absolute() {
        normalized = normalized.replace('/', "\\");
    }
    if Path::new(&normalized).parent().is_some() {
        normalized.trim_end_matches(['\\', '/']).to_owned()
    } else {
        normalized
    }
}

fn is_filesystem_panel_path(path: &str) -> bool {
    !path.is_empty()
        && !crate::domain::special_paths::is_virtual_path(path)
        && !crate::domain::file_entry::is_path_inside_archive(Path::new(path))
}

pub(super) fn has_panel_tab_snapshot_for_path(app: &ImageViewerApp, path: &Path) -> bool {
    let normalized_path = normalize_panel_path(path);
    app.tab_manager.tabs.iter().any(|outer_tab| {
        outer_tab
            .dual_panel_inactive_state
            .as_ref()
            .is_some_and(|snapshot| panel_path_matches_folder(&snapshot.path, &normalized_path))
            || outer_tab
                .dual_panel_tabs
                .as_ref()
                .is_some_and(|panel_tabs| {
                    panel_tabs
                        .left
                        .tabs
                        .iter()
                        .chain(panel_tabs.right.tabs.iter())
                        .any(|tab| panel_path_matches_folder(&tab.snapshot.path, &normalized_path))
                })
    })
}

fn is_live_panel_tab(
    is_active_workspace: bool,
    dual_panel_enabled: bool,
    focused_panel: crate::app::dual_panel::ActivePanel,
    physical_panel: crate::app::dual_panel::ActivePanel,
    tab_index: usize,
    active_tab_index: usize,
) -> bool {
    is_active_workspace
        && if dual_panel_enabled {
            tab_index == active_tab_index
        } else {
            physical_panel == focused_panel && tab_index == active_tab_index
        }
}

fn should_invalidate_panel_tab_listing(
    panel_path: &str,
    is_live_tab: bool,
    path_matches: &mut impl FnMut(&str) -> bool,
) -> bool {
    !is_live_tab && path_matches(panel_path)
}

fn invalidate_snapshot_listing(snapshot: &mut PanelSnapshot) {
    snapshot
        .miller_columns
        .invalidate(Path::new(&snapshot.path));

    if snapshot.items.is_empty()
        && snapshot.all_items.is_empty()
        && snapshot.loaded_path.is_empty()
        && !snapshot.is_loading_folder
        && snapshot.folder_load_error.is_none()
        && snapshot.renaming_state.is_none()
        && !snapshot.focus_rename
    {
        return;
    }

    snapshot
        .folder_load_generation
        .fetch_add(1, Ordering::Relaxed);
    snapshot.generation = crate::app::operations::folder_loading::allocate_folder_load_generation();
    snapshot.folder_load_generation =
        Arc::new(std::sync::atomic::AtomicUsize::new(snapshot.generation));

    snapshot.items = Arc::new(Vec::new());
    snapshot.all_items = Arc::new(Vec::new());
    snapshot.items_revision = snapshot.items_revision.wrapping_add(1);
    snapshot.items_snapshot_compact = false;
    snapshot.total_items = 0;
    snapshot.is_loading_folder = false;
    snapshot.folder_load_error = None;
    snapshot.loaded_path.clear();
    snapshot.pending_all_items_clear = false;
    snapshot.hold_visible_items_until_load_complete = false;
    snapshot.pending_items_rebuild = false;
    snapshot.pending_items_count = 0;
    snapshot.inactive_final_items_rebuild_pending = false;
    snapshot.stale_items_snapshot = None;
    snapshot.renaming_state = None;
    snapshot.focus_rename = false;

    snapshot.selected_item = None;
    snapshot.selected_file = None;
    snapshot.multi_selection.clear();
    snapshot.selection_anchor = None;
    snapshot.rectangle_selection_state = None;
    snapshot.selected_thumbnail = None;
    snapshot.selected_metadata = None;
    snapshot.selected_gif = None;
    snapshot.scroll_to_selected = false;
    snapshot.visible_index_range = None;
    snapshot.visible_paths_cache.clear();
    snapshot.visible_group_paths.clear();
    snapshot.visible_range_cached = None;
    snapshot.group_projection = Arc::new(Default::default());
}

pub(super) fn invalidate_cached_panel_tab_listings(
    app: &mut ImageViewerApp,
    normalized_folder: &str,
) {
    let normalized_folder = normalize_panel_path(Path::new(normalized_folder));
    invalidate_cached_panel_tab_listings_matching(app, |panel_path| {
        normalize_panel_path(Path::new(panel_path)) == normalized_folder
    });
}

pub(super) fn invalidate_cached_panel_tab_listings_for_paths(
    app: &mut ImageViewerApp,
    paths: &[PathBuf],
) {
    let normalized_paths: HashSet<String> = paths
        .iter()
        .map(|path| normalize_panel_path(path))
        .collect();
    if normalized_paths.is_empty() {
        return;
    }

    invalidate_cached_panel_tab_listings_matching(app, |panel_path| {
        normalized_paths.contains(&normalize_panel_path(Path::new(panel_path)))
    });
}

pub(super) fn invalidate_all_inactive_panel_tab_listings(app: &mut ImageViewerApp) {
    let active_workspace_id = app.tab_manager.active().id;
    let dual_panel_enabled = app.dual_panel_enabled;
    let focused_panel = app.dual_panel_active;
    let mut paths = std::collections::HashMap::<String, PathBuf>::new();

    for outer_tab in &app.tab_manager.tabs {
        let is_active_workspace = outer_tab.id == active_workspace_id;
        if let Some(snapshot) = outer_tab.dual_panel_inactive_state.as_ref() {
            let is_live_snapshot = is_active_workspace && dual_panel_enabled;
            if !is_live_snapshot && is_filesystem_panel_path(&snapshot.path) {
                paths
                    .entry(normalize_panel_path(Path::new(&snapshot.path)))
                    .or_insert_with(|| PathBuf::from(&snapshot.path));
            }
        }

        if let Some(panel_tabs) = outer_tab.dual_panel_tabs.as_ref() {
            for (physical_panel, panel) in [
                (crate::app::dual_panel::ActivePanel::Left, &panel_tabs.left),
                (
                    crate::app::dual_panel::ActivePanel::Right,
                    &panel_tabs.right,
                ),
            ] {
                for (index, panel_tab) in panel.tabs.iter().enumerate() {
                    if !is_live_panel_tab(
                        is_active_workspace,
                        dual_panel_enabled,
                        focused_panel,
                        physical_panel,
                        index,
                        panel.active_tab,
                    ) && is_filesystem_panel_path(&panel_tab.snapshot.path)
                    {
                        paths
                            .entry(normalize_panel_path(Path::new(&panel_tab.snapshot.path)))
                            .or_insert_with(|| PathBuf::from(&panel_tab.snapshot.path));
                    }
                }
            }
        }
    }

    for path in paths.values() {
        app.directory_dirty_registry.mark_dirty(path);
        app.directory_cache.invalidate(path);
        if let Some(directory_index) = &app.directory_index {
            let _ = directory_index.invalidate(path);
        }
        app.invalidate_folder_size_cache(path);
    }

    invalidate_cached_panel_tab_listings_matching(app, |_| true);
}

fn invalidate_cached_panel_tab_listings_matching(
    app: &mut ImageViewerApp,
    mut path_matches: impl FnMut(&str) -> bool,
) {
    let active_workspace_id = app.tab_manager.active().id;
    let dual_panel_enabled = app.dual_panel_enabled;
    let focused_panel = app.dual_panel_active;

    for outer_tab in &mut app.tab_manager.tabs {
        let is_active_workspace = outer_tab.id == active_workspace_id;
        if let Some(snapshot) = outer_tab.dual_panel_inactive_state.as_mut() {
            let is_live_snapshot = is_active_workspace && dual_panel_enabled;
            if should_invalidate_panel_tab_listing(
                &snapshot.path,
                is_live_snapshot,
                &mut path_matches,
            ) {
                invalidate_snapshot_listing(snapshot);
            }
        }

        if let Some(panel_tabs) = outer_tab.dual_panel_tabs.as_mut() {
            for (physical_panel, panel) in [
                (
                    crate::app::dual_panel::ActivePanel::Left,
                    &mut panel_tabs.left,
                ),
                (
                    crate::app::dual_panel::ActivePanel::Right,
                    &mut panel_tabs.right,
                ),
            ] {
                for (index, panel_tab) in panel.tabs.iter_mut().enumerate() {
                    let is_live_tab = is_live_panel_tab(
                        is_active_workspace,
                        dual_panel_enabled,
                        focused_panel,
                        physical_panel,
                        index,
                        panel.active_tab,
                    );
                    if should_invalidate_panel_tab_listing(
                        &panel_tab.snapshot.path,
                        is_live_tab,
                        &mut path_matches,
                    ) {
                        invalidate_snapshot_listing(&mut panel_tab.snapshot);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        is_live_panel_tab, panel_path_matches_folder, should_invalidate_panel_tab_listing,
    };
    use crate::app::dual_panel::ActivePanel;
    use crate::app::state::ImageViewerApp;
    use std::path::Path;

    #[test]
    fn inner_panel_folder_matching_normalizes_case_and_trailing_separator() {
        let normalized =
            ImageViewerApp::normalize_for_match(Path::new(r"C:\Users\user\Documents\Project"));

        assert!(panel_path_matches_folder(
            r"c:\users\user\documents\project\",
            &normalized
        ));
        assert!(panel_path_matches_folder(
            "C:/Users/user/Documents/Project/",
            &normalized
        ));
        assert!(!panel_path_matches_folder(
            r"C:\Users\user\Documents\Project 2",
            &normalized
        ));
        assert!(panel_path_matches_folder(
            r"C:\Temp\Folder.ext\",
            r"c:\temp\folder.ext"
        ));
        assert!(!panel_path_matches_folder(r"C:\", "C:"));
    }

    #[test]
    fn watcher_invalidation_preserves_the_live_tab_and_matches_inactive_tabs() {
        let normalized = ImageViewerApp::normalize_for_match(Path::new(r"C:\Temp\Folder"));

        assert!(is_live_panel_tab(
            true,
            true,
            ActivePanel::Left,
            ActivePanel::Left,
            0,
            0,
        ));
        assert!(is_live_panel_tab(
            true,
            true,
            ActivePanel::Left,
            ActivePanel::Right,
            0,
            0,
        ));
        assert!(!is_live_panel_tab(
            true,
            true,
            ActivePanel::Left,
            ActivePanel::Left,
            1,
            0,
        ));
        let mut matches_folder = |path: &str| panel_path_matches_folder(path, &normalized);
        assert!(!should_invalidate_panel_tab_listing(
            r"C:\Temp\Folder",
            true,
            &mut matches_folder,
        ));
        assert!(should_invalidate_panel_tab_listing(
            r"C:\Temp\Folder",
            false,
            &mut matches_folder,
        ));
        assert!(!should_invalidate_panel_tab_listing(
            r"C:\Temp\Other",
            false,
            &mut matches_folder,
        ));
    }

    #[test]
    fn single_panel_mode_only_preserves_the_focused_panel_tab() {
        assert!(is_live_panel_tab(
            true,
            false,
            ActivePanel::Right,
            ActivePanel::Right,
            1,
            1,
        ));
        assert!(!is_live_panel_tab(
            true,
            false,
            ActivePanel::Right,
            ActivePanel::Left,
            1,
            1,
        ));
        assert!(!is_live_panel_tab(
            false,
            false,
            ActivePanel::Right,
            ActivePanel::Right,
            1,
            1,
        ));
    }
}
