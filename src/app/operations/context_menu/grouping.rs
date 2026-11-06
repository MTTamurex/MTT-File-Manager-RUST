use crate::application::context_menu::ContextMenuItem;
use crate::domain::file_entry::GroupMode;
use rust_i18n::t;

pub(super) fn menu(current: GroupMode, descending: bool, enabled: bool) -> ContextMenuItem {
    let mut sub_items = Vec::new();
    for (index, mode, label) in [
        (0, GroupMode::None, t!("secondary_toolbar.group_none")),
        (1, GroupMode::Name, t!("secondary_toolbar.group_name")),
        (2, GroupMode::Date, t!("secondary_toolbar.group_date")),
        (3, GroupMode::Type, t!("secondary_toolbar.group_type")),
        (4, GroupMode::Size, t!("secondary_toolbar.group_size")),
    ] {
        sub_items.push(
            ContextMenuItem::new(-300 - index, label)
                .with_command(format!("group:{}", mode.preference_value()))
                .checked(current == mode)
                .enabled(enabled),
        );
    }
    sub_items.push(ContextMenuItem::separator());
    for (id, value, label) in [
        (-306, false, t!("grouping.ascending")),
        (-307, true, t!("grouping.descending")),
    ] {
        sub_items.push(
            ContextMenuItem::new(id, label)
                .with_command(if value {
                    "group_direction:descending"
                } else {
                    "group_direction:ascending"
                })
                .checked(descending == value)
                .enabled(enabled && current != GroupMode::None),
        );
    }
    ContextMenuItem::new(-299, t!("secondary_toolbar.group_by"))
        .with_svg_icon("group")
        .with_subitems(sub_items)
        .enabled(enabled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grouping_choices_and_direction_follow_availability() {
        for enabled in [false, true] {
            let menu = menu(GroupMode::Type, true, enabled);
            assert_eq!(menu.is_enabled, enabled);
            assert!(menu
                .sub_items
                .iter()
                .filter(|item| !item.is_separator)
                .all(|item| item.is_enabled == enabled));
            let checked: Vec<_> = menu
                .sub_items
                .iter()
                .filter(|item| item.is_checked)
                .map(|item| item.command_string.as_deref().unwrap())
                .collect();
            assert_eq!(checked, ["group:type", "group_direction:descending"]);
        }
    }

    #[test]
    fn ungrouped_view_allows_grouping_but_not_direction_changes() {
        let menu = menu(GroupMode::None, false, true);
        for item in menu.sub_items.iter().filter(|item| !item.is_separator) {
            let command = item.command_string.as_deref().unwrap();
            assert_eq!(item.is_enabled, command.starts_with("group:"));
        }
    }
}
