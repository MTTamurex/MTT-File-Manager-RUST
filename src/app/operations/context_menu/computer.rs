use crate::application::context_menu::{ContextMenuItem, ContextMenuOrigin};
use crate::domain::special_paths::COMPUTER_VIEW_ID;
use rust_i18n::t;
use std::path::{Path, PathBuf};

pub(super) fn is_sidebar_computer(origin: ContextMenuOrigin, paths: &[PathBuf]) -> bool {
    origin == ContextMenuOrigin::SidebarTree
        && paths.len() == 1
        && paths[0] == Path::new(COMPUTER_VIEW_ID)
}

pub(super) fn network_items(map_pending: bool, location_pending: bool) -> [ContextMenuItem; 2] {
    [
        ContextMenuItem::new(-89, t!("context_menu.map_network_drive"))
            .with_command("map_network_drive")
            .with_svg_icon("drive")
            .enabled(!map_pending),
        ContextMenuItem::new(-84, t!("context_menu.add_network_location"))
            .with_command("add_network_location")
            .with_svg_icon("folder_new")
            .enabled(!location_pending),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidebar_computer_menu_does_not_depend_on_the_current_folder() {
        let paths = [PathBuf::from(COMPUTER_VIEW_ID)];
        assert!(is_sidebar_computer(ContextMenuOrigin::SidebarTree, &paths));
        assert!(!is_sidebar_computer(ContextMenuOrigin::FileView, &paths));
        assert!(!is_sidebar_computer(
            ContextMenuOrigin::GlobalSearch,
            &paths
        ));
        assert!(!is_sidebar_computer(ContextMenuOrigin::SidebarTree, &[]));
        assert!(!is_sidebar_computer(
            ContextMenuOrigin::SidebarTree,
            &[PathBuf::from("C:\\")]
        ));
    }

    #[test]
    fn computer_network_commands_preserve_independent_pending_states() {
        for map_pending in [false, true] {
            for location_pending in [false, true] {
                let [map, location] = network_items(map_pending, location_pending);
                assert_eq!(map.command_string.as_deref(), Some("map_network_drive"));
                assert_eq!(
                    location.command_string.as_deref(),
                    Some("add_network_location")
                );
                assert_eq!(map.is_enabled, !map_pending);
                assert_eq!(location.is_enabled, !location_pending);
            }
        }
    }
}
