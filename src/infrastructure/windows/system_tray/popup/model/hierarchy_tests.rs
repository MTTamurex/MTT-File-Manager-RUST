use super::super::super::tests::NativeWindow;
use super::tests::snapshot;
use super::*;

#[test]
fn a_regular_tab_after_a_split_workspace_is_not_visually_nested_in_the_right_panel() {
    let native = NativeWindow::new();
    let mut snapshot = snapshot();
    let mut regular = snapshot.workspaces[0].clone();
    regular.tab.id = 8;
    regular.tab.path = "Other".into();
    regular.panels = None;
    snapshot.workspaces.push(regular);
    let mut model = Model::new(vec![(WindowRef::capture(native.0).unwrap(), snapshot)]);
    model.toggle(0);
    let separator = model
        .rows
        .iter()
        .position(|row| matches!(row.action, Action::Separator))
        .unwrap();
    let panel_tab = &model.rows[separator - 1];
    let regular = &model.rows[separator + 1];
    assert_eq!(panel_tab.indent, 2);
    assert!(matches!(
        panel_tab.action,
        Action::Command(Command::Activate(
            _,
            Target {
                workspace: 7,
                panel: Some((true, 12)),
            }
        ))
    ));
    assert_eq!(regular.indent, 1);
    assert_eq!(regular.title, "Other");
    assert!(matches!(
        regular.action,
        Action::Command(Command::Activate(
            _,
            Target {
                workspace: 8,
                panel: None,
            }
        ))
    ));
    assert!(!model.rows[separator].selectable());
    assert_eq!(model.rows[separator].height, 8);
}

#[test]
fn separators_mark_workspace_boundaries_not_left_right_panel_boundaries() {
    let native = NativeWindow::new();
    let mut snapshot = snapshot();
    let split = snapshot.workspaces[0].clone();
    snapshot.workspaces.clear();
    for (index, dual) in [false, true, true, false].into_iter().enumerate() {
        let mut workspace = split.clone();
        workspace.tab.id = index + 1;
        if !dual {
            workspace.panels = None;
        }
        snapshot.workspaces.push(workspace);
    }
    let mut model = Model::new(vec![(WindowRef::capture(native.0).unwrap(), snapshot)]);
    model.toggle(0);
    let separators: Vec<_> = model
        .rows
        .iter()
        .enumerate()
        .filter_map(|(index, row)| matches!(row.action, Action::Separator).then_some(index))
        .collect();
    assert_eq!(separators.len(), 3);
    let mut group = 1;
    for row in model.rows.iter().skip(1) {
        match &row.action {
            Action::Separator => group += 1,
            Action::Command(Command::Activate(_, target)) => assert_eq!(target.workspace, group),
            Action::Information => assert_eq!(row.indent, 1),
            _ => panic!("unexpected instance command inside a workspace"),
        }
    }
    assert!(!matches!(model.rows[1].action, Action::Separator));
    assert!(!matches!(
        model.rows.last().unwrap().action,
        Action::Separator
    ));
}
