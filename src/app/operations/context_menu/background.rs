//! App-owned background menu actions, independent of Windows Explorer's view.
use crate::app::ImageViewerApp;
use crate::application::context_menu::ContextMenuItem;
use crate::domain::file_entry::{SortMode, ViewMode};
use rust_i18n::t;

pub(super) fn is_owned_shell_item(verb: Option<&str>, text: &str) -> bool {
    if verb.is_some_and(|verb| {
        matches!(
            verb.to_ascii_lowercase().as_str(),
            "view" | "arrange" | "sortby" | "groupby" | "refresh" | "cmd" | "cmdadmin"
        )
    }) {
        return true;
    }
    [
        t!("context_menu.view"),
        t!("context_menu.sort_by"),
        t!("context_menu.refresh"),
        t!("context_menu.open_cmd"),
        t!("context_menu.open_cmd_admin"),
        t!("secondary_toolbar.group_by"),
    ]
    .iter()
    .any(|label| text.trim().eq_ignore_ascii_case(label))
}

pub(crate) fn view_items(app: &ImageViewerApp) -> Vec<ContextMenuItem> {
    vec![
        view_menu(app.view_mode, app.current_folder_locked),
        sort_menu(
            sort_choices(app),
            app.sort_mode,
            app.sort_descending,
            app.current_folder_locked,
        ),
    ]
}

fn view_menu(current: ViewMode, locked: bool) -> ContextMenuItem {
    let modes = [
        (ViewMode::List, t!("secondary_toolbar.details")),
        (ViewMode::ColumnList, t!("secondary_toolbar.column_list")),
        (ViewMode::Grid, t!("secondary_toolbar.grid")),
        (ViewMode::Miller, t!("secondary_toolbar.miller_columns")),
    ];
    ContextMenuItem::new(-400, t!("context_menu.view"))
        .with_svg_icon("view_grid")
        .enabled(!locked)
        .with_subitems(
            modes
                .into_iter()
                .enumerate()
                .map(|(index, (mode, label))| {
                    ContextMenuItem::new(-410 - index as i32, label)
                        .with_command(format!("view:{}", mode.preference_value()))
                        .checked(current == mode)
                        .enabled(!locked)
                })
                .collect(),
        )
}

fn sort_menu(
    choices: Vec<(SortMode, String)>,
    current: SortMode,
    descending: bool,
    locked: bool,
) -> ContextMenuItem {
    let mut sort_items: Vec<_> = choices
        .into_iter()
        .enumerate()
        .map(|(index, (mode, label))| {
            ContextMenuItem::new(-10000 - index as i32, label)
                .with_command(format!("sort:{}", sort_value(mode)))
                .checked(current == mode)
                .enabled(!locked)
        })
        .collect();
    sort_items.push(ContextMenuItem::separator());
    for (id, choice_descending, label) in [
        (-401, false, t!("grouping.ascending")),
        (-402, true, t!("grouping.descending")),
    ] {
        sort_items.push(
            ContextMenuItem::new(id, label)
                .with_command(format!("sort_direction:{choice_descending}"))
                .checked(descending == choice_descending)
                .enabled(!locked),
        );
    }
    ContextMenuItem::new(-403, t!("context_menu.sort_by"))
        .with_svg_icon("sort")
        .with_subitems(sort_items)
        .enabled(!locked)
}

pub(crate) fn refresh_item(app: &ImageViewerApp) -> ContextMenuItem {
    ContextMenuItem::new(-404, t!("context_menu.refresh"))
        .with_command("refresh_view")
        .with_svg_icon("refresh")
        .with_shortcut(
            app.shortcuts
                .label(crate::app::shortcuts::ShortcutAction::Refresh),
        )
}

pub(super) fn folder_items(app: &ImageViewerApp) -> Vec<ContextMenuItem> {
    vec![
        ContextMenuItem::new(-405, t!("context_menu.open_cmd"))
            .with_command("open_cmd")
            .with_svg_icon("terminal"),
        ContextMenuItem::new(-406, t!("context_menu.open_cmd_admin"))
            .with_command("open_cmd_admin")
            .with_svg_icon("terminal"),
        ContextMenuItem::separator(),
        ContextMenuItem::new(-407, t!("context_menu.properties"))
            .with_command("folder_properties")
            .with_svg_icon("properties")
            .with_shortcut(
                app.shortcuts
                    .label(crate::app::shortcuts::ShortcutAction::Properties),
            ),
    ]
}

fn sort_choices(app: &ImageViewerApp) -> Vec<(SortMode, String)> {
    let mut choices = vec![(
        SortMode::Name,
        t!("secondary_toolbar.sort_name").to_string(),
    )];
    choices.extend(base_sort_choices(app.navigation_state.is_computer_view));
    choices.extend(
        crate::app::column_state::choices::for_app(app)
            .into_iter()
            .map(|(id, label)| (SortMode::Column(id), label)),
    );
    choices
}

fn base_sort_choices(computer: bool) -> Vec<(SortMode, String)> {
    let modes = if computer {
        vec![
            (
                SortMode::DriveTotalSpace,
                t!("secondary_toolbar.sort_total_space"),
            ),
            (
                SortMode::DriveFreeSpace,
                t!("secondary_toolbar.sort_free_space"),
            ),
            (
                SortMode::DriveLetter,
                t!("secondary_toolbar.sort_drive_letter"),
            ),
        ]
    } else {
        vec![
            (SortMode::Date, t!("secondary_toolbar.sort_date")),
            (SortMode::Size, t!("secondary_toolbar.sort_size")),
            (SortMode::Type, t!("secondary_toolbar.sort_type")),
        ]
    };
    modes
        .into_iter()
        .map(|(mode, label)| (mode, label.to_string()))
        .collect()
}

fn sort_value(mode: SortMode) -> String {
    match mode {
        SortMode::Name => "name".into(),
        SortMode::Date => "date".into(),
        SortMode::Size => "size".into(),
        SortMode::Type => "type".into(),
        SortMode::DriveTotalSpace => "total".into(),
        SortMode::DriveFreeSpace => "free".into(),
        SortMode::DriveLetter => "letter".into(),
        SortMode::Column(id) => format!(
            "column:{}",
            serde_json::to_string(&id).expect("column identity")
        ),
    }
}

fn parse_sort(value: &str) -> Option<SortMode> {
    Some(match value {
        "name" => SortMode::Name,
        "date" => SortMode::Date,
        "size" => SortMode::Size,
        "type" => SortMode::Type,
        "total" => SortMode::DriveTotalSpace,
        "free" => SortMode::DriveFreeSpace,
        "letter" => SortMode::DriveLetter,
        _ => SortMode::Column(serde_json::from_str(value.strip_prefix("column:")?).ok()?),
    })
}

pub(crate) fn handle_view_command(app: &mut ImageViewerApp, command: &str) -> bool {
    if command == "refresh_view" {
        app.trigger_manual_refresh();
    } else if let Some(value) = command.strip_prefix("view:") {
        if ["grid", "list", "column_list", "miller"].contains(&value) {
            app.change_view_mode(ViewMode::from_preference(value));
        }
    } else if let Some(value) = command.strip_prefix("sort:") {
        if let Some(mode) = parse_sort(value)
            .filter(|mode| sort_choices(app).iter().any(|(choice, _)| choice == mode))
        {
            app.change_sort_mode(mode);
        }
    } else if let Some(value) = command.strip_prefix("sort_direction:") {
        if let Ok(descending) = value.parse::<bool>() {
            app.change_sort_direction(descending);
        }
    } else {
        return false;
    }
    true
}

impl ImageViewerApp {
    pub(crate) fn change_sort_mode(&mut self, mode: SortMode) {
        if self.current_folder_locked {
            return;
        }
        self.sort_mode = mode;
        if self.navigation_state.is_computer_view {
            self.sort_mode_computer = mode;
        } else {
            self.sort_mode_normal = mode;
        }
        self.sort_items();
        self.save_preferences();
    }

    pub(crate) fn change_sort_direction(&mut self, descending: bool) {
        if self.current_folder_locked {
            return;
        }
        self.sort_descending = descending;
        self.sort_descending_normal = descending;
        self.sort_items();
        self.save_preferences();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::list_columns::{ColumnId, PropertyId};

    #[test]
    fn sort_commands_roundtrip_including_shell_properties() {
        for mode in [
            SortMode::Name,
            SortMode::Date,
            SortMode::Size,
            SortMode::Type,
            SortMode::DriveTotalSpace,
            SortMode::DriveFreeSpace,
            SortMode::DriveLetter,
            SortMode::Column(ColumnId::Tags),
            SortMode::Column(ColumnId::OriginalLocation),
            SortMode::Column(ColumnId::Property(PropertyId {
                fmtid: u128::MAX,
                pid: 3,
            })),
        ] {
            assert_eq!(parse_sort(&sort_value(mode)), Some(mode));
        }
        assert_eq!(parse_sort("invalid"), None);
        assert_eq!(parse_sort("column:invalid"), None);
    }

    #[test]
    fn drive_and_file_sort_choices_match_their_scope() {
        assert_eq!(
            base_sort_choices(true)
                .iter()
                .map(|(mode, _)| *mode)
                .collect::<Vec<_>>(),
            [
                SortMode::DriveTotalSpace,
                SortMode::DriveFreeSpace,
                SortMode::DriveLetter
            ]
        );
        assert_eq!(
            base_sort_choices(false)
                .iter()
                .map(|(mode, _)| *mode)
                .collect::<Vec<_>>(),
            [SortMode::Date, SortMode::Size, SortMode::Type]
        );
    }

    #[test]
    fn filters_only_background_actions_owned_by_the_app() {
        for verb in ["VIEW", "arrange", "groupby", "refresh", "cmd", "cmdadmin"] {
            assert!(is_owned_shell_item(Some(verb), "Localized label"));
        }
        assert!(!is_owned_shell_item(Some("new"), "New"));
        assert!(!is_owned_shell_item(Some("runas"), "Third-party action"));
        assert!(!is_owned_shell_item(None, "Third-party action"));
    }

    #[test]
    fn view_choices_show_the_current_mode_and_follow_folder_locks() {
        for locked in [false, true] {
            let menu = view_menu(ViewMode::ColumnList, locked);
            assert_eq!(menu.is_enabled, !locked);
            assert_eq!(menu.sub_items.len(), 4);
            assert_eq!(
                menu.sub_items.iter().filter(|item| item.is_checked).count(),
                1
            );
            assert!(menu.sub_items.iter().all(|item| item.is_enabled == !locked));
            assert_eq!(
                menu.sub_items
                    .iter()
                    .find(|item| item.is_checked)
                    .unwrap()
                    .command_string
                    .as_deref(),
                Some("view:column_list")
            );
        }
    }

    #[test]
    fn sorting_checks_the_current_criterion_and_direction_without_enabling_locked_children() {
        for locked in [false, true] {
            let menu = sort_menu(base_sort_choices(false), SortMode::Size, true, locked);
            assert_eq!(menu.is_enabled, !locked);
            let checked: Vec<_> = menu
                .sub_items
                .iter()
                .filter(|item| item.is_checked)
                .map(|item| item.command_string.as_deref().unwrap())
                .collect();
            assert_eq!(checked, ["sort:size", "sort_direction:true"]);
            assert!(menu
                .sub_items
                .iter()
                .filter(|item| !item.is_separator)
                .all(|item| item.is_enabled == !locked));
        }
    }
}
