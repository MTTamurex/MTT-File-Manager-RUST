//! Labels must describe the same Shift-dependent delete action that is dispatched.
use crate::app::shortcuts::{ShortcutAction, ShortcutBindings};
use crate::application::context_menu::ContextMenuItem;
use rust_i18n::t;

pub(super) fn update_menu_labels(
    items: &mut [ContextMenuItem],
    permanently: bool,
    shortcuts: &ShortcutBindings,
) {
    for item in items {
        if matches!(item.id, -6 | -34) {
            item.text = if permanently {
                t!("context_menu.delete_permanent").to_string()
            } else {
                t!("context_menu.delete").to_string()
            };
            item.keyboard_shortcut = Some(shortcuts.label(if permanently {
                ShortcutAction::DeletePermanently
            } else {
                ShortcutAction::Delete
            }));
        }
    }
}

pub(super) fn toolbar_tooltip(permanently: bool) -> String {
    if permanently {
        t!("context_menu.delete_permanent").to_string()
    } else {
        t!("secondary_toolbar.delete").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shift_changes_only_managed_delete_labels_and_restores_them_on_release() {
        let shortcuts = ShortcutBindings::default();
        let mut items = vec![
            ContextMenuItem::primary(-6, "Delete").with_command("delete"),
            ContextMenuItem::new(1, "Shell action"),
        ];
        for permanently in [false, true, false] {
            update_menu_labels(&mut items, permanently, &shortcuts);
            let key = if permanently {
                "context_menu.delete_permanent"
            } else {
                "context_menu.delete"
            };
            assert_eq!(items[0].text, t!(key));
            assert_eq!(
                items[0].keyboard_shortcut,
                Some(shortcuts.label(if permanently {
                    ShortcutAction::DeletePermanently
                } else {
                    ShortcutAction::Delete
                }))
            );
            assert_eq!(items[0].id, -6);
            assert_eq!(items[0].command_string.as_deref(), Some("delete"));
            assert_eq!(items[1].text, "Shell action");
        }
        assert_eq!(toolbar_tooltip(true), t!("context_menu.delete_permanent"));
        assert_eq!(toolbar_tooltip(false), t!("secondary_toolbar.delete"));
    }
}
