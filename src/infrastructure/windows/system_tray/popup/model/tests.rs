use super::super::super::{
    model::{Entry, Workspace},
    tests::NativeWindow,
};
use super::*;

fn snapshot() -> Snapshot {
    Snapshot {
        workspaces: vec![Workspace {
            tab: Entry {
                id: 7,
                path: "Example".into(),
                icon: LocationKind::Folder,
            },
            panels: Some([
                vec![Entry {
                    id: 11,
                    path: "Left".into(),
                    icon: LocationKind::Folder,
                }],
                vec![Entry {
                    id: 12,
                    path: "Tag: Example".into(),
                    icon: LocationKind::Tag,
                }],
            ]),
        }],
        active_path: "Example".into(),
        active_icon: LocationKind::Tag,
        hidden_at: Some("2026-01-01 12:30:00".into()),
    }
}

#[test]
fn expansion_preserves_workspace_panel_ids_icons_and_hidden_timestamp() {
    let native = NativeWindow::new();
    let window = WindowRef::capture(native.0).unwrap();
    let mut model = Model::new(vec![(window, snapshot())]);
    assert_eq!(model.rows.len(), 1);
    assert_eq!(model.rows[0].icon, Some(LocationKind::Tag));
    model.toggle(0);
    assert_eq!(model.rows[0].expanded, Some(true));
    assert_eq!(
        model.rows[0].sent_at.as_deref(),
        Some("2026-01-01 12:30:00")
    );
    assert!(!model
        .rows
        .iter()
        .skip(1)
        .any(|row| row.title == "Example" || row.sent_at.is_some()));
    let targets: Vec<_> = model
        .rows
        .iter()
        .filter_map(|row| match &row.action {
            Action::Command(Command::Activate(_, target)) => Some(target.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        targets,
        [
            Target {
                workspace: 7,
                panel: Some((false, 11))
            },
            Target {
                workspace: 7,
                panel: Some((true, 12))
            }
        ]
    );
    assert!(!model
        .rows
        .iter()
        .any(|row| matches!(row.action, Action::Command(Command::Exit(_)))));
    model.toggle(0);
    assert_eq!(model.rows.len(), 1);
}

#[test]
fn separate_instances_expand_independently_and_expose_their_own_window() {
    let first = NativeWindow::new();
    let second = NativeWindow::new();
    let windows =
        [&first, &second].map(|native| (WindowRef::capture(native.0).unwrap(), snapshot()));
    let mut model = Model::new(windows.into());
    model.toggle(0);
    let second_index = model.rows.iter().position(|row| matches!(row.action, Action::Command(Command::Open(window)) if window.hwnd == second.0)).unwrap();
    model.toggle(second_index);
    assert_eq!(model.expanded.len(), 2);
    let headers: Vec<_> = model.rows.iter().filter_map(Row::instance).collect();
    assert_eq!(
        headers.iter().map(|window| window.hwnd).collect::<Vec<_>>(),
        [first.0, second.0]
    );
    assert!(!model
        .rows
        .iter()
        .any(|row| matches!(row.action, Action::Command(Command::Exit(_)))));
}

#[test]
fn a_single_non_split_tab_has_no_expansion_or_duplicate_location() {
    let native = NativeWindow::new();
    let mut snapshot = snapshot();
    snapshot.workspaces[0].panels = None;
    let mut model = Model::new(vec![(WindowRef::capture(native.0).unwrap(), snapshot)]);
    assert_eq!(model.rows.len(), 1);
    assert_eq!(model.rows[0].expanded, None);
    assert_eq!(model.rows[0].subtitle, "Example");
    assert_eq!(
        model.rows[0].sent_at.as_deref(),
        Some("2026-01-01 12:30:00")
    );
    model.toggle(0);
    assert_eq!(model.rows.len(), 1);
    assert!(model.expanded.is_empty());
}

#[test]
fn multiple_non_split_tabs_keep_all_activation_targets_without_timestamp_rows() {
    let native = NativeWindow::new();
    let mut snapshot = snapshot();
    snapshot.hidden_at = None;
    snapshot.workspaces[0].panels = None;
    let mut second = snapshot.workspaces[0].clone();
    second.tab.id = 8;
    second.tab.path = "Other".into();
    snapshot.workspaces.push(second);
    let mut model = Model::new(vec![(WindowRef::capture(native.0).unwrap(), snapshot)]);
    assert_eq!(model.rows[0].expanded, Some(false));
    assert!(model.rows[0].sent_at.is_none());
    model.toggle(0);
    assert_eq!(model.rows.len(), 3);
    let targets: Vec<_> = model
        .rows
        .iter()
        .filter_map(|row| match &row.action {
            Action::Command(Command::Activate(_, target)) => Some(target.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        targets,
        [
            Target {
                workspace: 7,
                panel: None
            },
            Target {
                workspace: 8,
                panel: None
            }
        ]
    );
}

#[test]
fn keyboard_navigation_skips_information_wraps_and_keeps_selection_visible() {
    let native = NativeWindow::new();
    let mut model = Model::new(vec![(WindowRef::capture(native.0).unwrap(), snapshot())]);
    model.toggle(0);
    model.navigate(1);
    assert!(model.rows[model.selected.unwrap()].selectable());
    assert_ne!(model.selected, Some(1));
    for _ in 0..model.rows.len() + 1 {
        model.navigate(1);
        model.reveal_selection(60);
        let index = model.selected.unwrap();
        if index < model.rows.len() {
            assert!(model.rows[index].selectable());
            let top: i32 = model.rows[..index].iter().map(|row| row.height).sum();
            assert!(top >= model.scroll);
        }
    }
    model.selected = Some(model.rows.len());
    model.navigate(1);
    assert_eq!(model.selected, Some(0));
    assert_eq!(model.row_at(-1), None);
    assert_eq!(model.row_at(model.content_height()), None);
}
