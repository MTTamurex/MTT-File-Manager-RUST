use crate::app::dual_panel::PanelSnapshot;

/// Restored group tabs have no listing or previous load request. Unlike an
/// already loaded empty folder, they must load when their workspace is selected.
pub(super) fn inactive_panel_needs_load(snapshot: &PanelSnapshot) -> bool {
    needs_load(
        snapshot.is_loading_folder,
        snapshot.loaded_path.is_empty(),
        snapshot.items.is_empty() && snapshot.all_items.is_empty(),
        snapshot.folder_load_error.is_some(),
    )
}

fn needs_load(loading: bool, never_requested: bool, empty: bool, failed: bool) -> bool {
    loading || (never_requested && empty && !failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unvisited_group_panel_loads_on_workspace_selection() {
        assert!(needs_load(false, true, true, false));
    }

    #[test]
    fn completed_empty_folder_does_not_reload_on_every_selection() {
        assert!(!needs_load(false, false, true, false));
        assert!(needs_load(true, false, true, false));
    }

    #[test]
    fn failed_or_populated_panel_is_not_retried_implicitly() {
        assert!(!needs_load(false, true, true, true));
        assert!(!needs_load(false, true, false, false));
    }
}
