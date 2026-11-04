use crate::application::context_menu::ContextMenuItem;
use rust_i18n::t;

pub(super) const MENU_ID: i32 = -110;

pub(super) fn menu(shortcut: String) -> ContextMenuItem {
    ContextMenuItem::new(MENU_ID, t!("context_menu.new"))
        .with_svg_icon("folder_new")
        .with_subitems(vec![ContextMenuItem::new(-1, t!("file_types.folder"))
            .with_svg_icon("folder")
            .with_shortcut(shortcut)])
}

/// Keep the app's creation action and shortcut, replacing every native duplicate.
pub(super) fn unify_folder(items: &mut Vec<ContextMenuItem>, folder: ContextMenuItem) {
    items.retain(|item| {
        item.id != -1
            && !item
                .command_string
                .as_deref()
                .is_some_and(|verb| verb.eq_ignore_ascii_case("newfolder"))
    });
    let mut previous_is_separator = true;
    items.retain(|item| {
        let keep = !item.is_separator || !previous_is_separator;
        previous_is_separator = item.is_separator;
        keep
    });
    while items.last().is_some_and(|item| item.is_separator) {
        items.pop();
    }
    items.insert(0, folder);
}

/// Replace the local fallback only when creation is permitted in this location.
pub(super) fn merge_shell(items: &mut [ContextMenuItem], mut shell_menu: ContextMenuItem) {
    // A disabled native menu must not disable the independently available app action.
    if !shell_menu.is_enabled {
        return;
    }
    let Some(local_menu) = items.iter_mut().find(|item| item.id == MENU_ID) else {
        return;
    };
    let Some(folder) = local_menu
        .sub_items
        .iter()
        .find(|item| item.id == -1)
        .cloned()
    else {
        return;
    };
    unify_folder(&mut shell_menu.sub_items, folder);
    shell_menu.text = t!("context_menu.new").to_string();
    if shell_menu.icon.is_none() {
        shell_menu.svg_icon_name = Some("folder_new".to_string());
    }
    *local_menu = shell_menu;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restricted_new_menu_uses_only_the_app_action() {
        let mut items = vec![
            ContextMenuItem::new(42, "Folder").with_command("newfolder"),
            ContextMenuItem::separator(),
        ];
        unify_folder(&mut items, menu("Ctrl+Shift+N".into()).sub_items.remove(0));
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, -1);
        assert_eq!(items[0].keyboard_shortcut.as_deref(), Some("Ctrl+Shift+N"));
    }

    #[test]
    fn full_new_menu_replaces_folder_and_keeps_shell_commands() {
        let mut items = vec![
            ContextMenuItem::new(42, "Folder").with_command("NEWFOLDER"),
            ContextMenuItem::new(43, "Text document").with_command("newtext"),
        ];
        unify_folder(
            &mut items,
            menu("Custom shortcut".into()).sub_items.remove(0),
        );
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id, -1);
        assert_eq!(items[1].id, 43);
        assert_eq!(items[1].command_string.as_deref(), Some("newtext"));
    }

    #[test]
    fn disabled_alternatives_stay_disabled() {
        let mut items = vec![
            ContextMenuItem::new(42, "Folder").with_command("newfolder"),
            ContextMenuItem::new(43, "Text document").enabled(false),
        ];
        unify_folder(&mut items, menu(String::new()).sub_items.remove(0));
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id, -1);
        assert!(!items[1].is_enabled);
    }

    #[test]
    fn local_fallback_is_usable_without_shell_results() {
        let item = menu("Ctrl+Shift+N".into());
        assert!(item.is_enabled);
        assert!(!item.is_loading_placeholder);
        assert_eq!(item.sub_items.len(), 1);
        assert_eq!(item.sub_items[0].id, -1);
    }

    #[test]
    fn shell_merge_preserves_local_action_and_pending_native_submenu() {
        let mut items = vec![menu("Custom shortcut".into())];
        let shell = ContextMenuItem {
            has_pending_submenu: true,
            ..ContextMenuItem::new(42, "New").with_command("new")
        };
        merge_shell(&mut items, shell);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, 42);
        assert!(items[0].has_pending_submenu);
        assert_eq!(items[0].sub_items[0].id, -1);
        assert_eq!(
            items[0].sub_items[0].keyboard_shortcut.as_deref(),
            Some("Custom shortcut")
        );
    }

    #[test]
    fn shell_merge_does_not_add_new_where_creation_is_not_permitted() {
        let mut items = vec![];
        merge_shell(&mut items, ContextMenuItem::new(42, "New"));
        assert!(items.is_empty());
    }

    #[test]
    fn disabled_shell_menu_keeps_the_available_local_fallback() {
        let mut items = vec![menu("Custom shortcut".into())];
        merge_shell(&mut items, ContextMenuItem::new(42, "New").enabled(false));
        assert_eq!(items[0].id, MENU_ID);
        assert!(items[0].is_enabled);
        assert!(items[0].sub_items[0].is_enabled);
        assert_eq!(items[0].sub_items[0].id, -1);
    }

    #[test]
    fn eagerly_loaded_shell_menu_is_unified_without_requesting_another_load() {
        let mut items = vec![menu("Custom shortcut".into())];
        let shell = ContextMenuItem::new(42, "New").with_subitems(vec![
            ContextMenuItem::new(43, "Folder").with_command("newfolder"),
            ContextMenuItem::new(44, "Text document").with_command("newtext"),
        ]);
        merge_shell(&mut items, shell);
        assert!(!items[0].has_pending_submenu);
        assert_eq!(items[0].sub_items.len(), 2);
        assert_eq!(items[0].sub_items[0].id, -1);
        assert_eq!(items[0].sub_items[1].id, 44);
    }

    #[test]
    fn repeated_submenu_results_never_duplicate_folder() {
        let mut items = menu(String::new()).sub_items;
        items.push(ContextMenuItem::new(42, "Folder").with_command("newfolder"));
        items.push(ContextMenuItem::separator());
        items.push(ContextMenuItem::separator());
        items.push(ContextMenuItem::new(43, "Text document"));
        let folder = items[0].clone();
        unify_folder(&mut items, folder.clone());
        unify_folder(&mut items, folder);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id, -1);
        assert_eq!(items[1].id, 43);
    }
}
