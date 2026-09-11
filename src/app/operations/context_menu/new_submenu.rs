use crate::application::context_menu::ContextMenuItem;

/// Avoid duplicating the main menu's New Folder action, but never empty a
/// restricted native New menu whose only available action is folder creation.
pub(super) fn remove_duplicate_folder(items: &mut Vec<ContextMenuItem>) {
    let is_folder = |item: &ContextMenuItem| {
        item.command_string
            .as_deref()
            .is_some_and(|verb| verb.eq_ignore_ascii_case("newfolder"))
    };
    if items
        .iter()
        .any(|item| !item.is_separator && item.is_enabled && !is_folder(item))
    {
        items.retain(|item| !is_folder(item));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restricted_new_menu_keeps_its_only_action() {
        let mut items = vec![
            ContextMenuItem::new(42, "Folder").with_command("newfolder"),
            ContextMenuItem::separator(),
        ];
        remove_duplicate_folder(&mut items);
        assert_eq!(items[0].id, 42);
        assert_eq!(items[0].command_string.as_deref(), Some("newfolder"));
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn full_new_menu_removes_duplicate_folder_and_keeps_shell_commands() {
        let mut items = vec![
            ContextMenuItem::new(42, "Folder").with_command("NEWFOLDER"),
            ContextMenuItem::new(43, "Text document").with_command("newtext"),
        ];
        remove_duplicate_folder(&mut items);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, 43);
        assert_eq!(items[0].command_string.as_deref(), Some("newtext"));
    }

    #[test]
    fn disabled_alternatives_do_not_remove_the_only_available_action() {
        let mut items = vec![
            ContextMenuItem::new(42, "Folder").with_command("newfolder"),
            ContextMenuItem::new(43, "Text document").enabled(false),
        ];
        remove_duplicate_folder(&mut items);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id, 42);
    }
}
