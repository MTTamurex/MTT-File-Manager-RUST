use super::super::super::tests::NativeWindow;
use super::tests::snapshot;
use super::*;

fn numbered_targets(model: &Model) -> Vec<(usize, Target)> {
    let mut number = 0;
    model
        .rows
        .iter()
        .filter_map(|row| {
            if let Some(current) = row.tab_number {
                number = current;
            }
            match &row.action {
                Action::Command(Command::Activate(_, target)) => Some((number, target.clone())),
                _ => None,
            }
        })
        .collect()
}

#[test]
fn numbering_follows_bar_order_after_reordering_without_changing_activation_ids() {
    let native = NativeWindow::new();
    let mut snapshot = snapshot();
    let mut regular = snapshot.workspaces[0].clone();
    regular.tab.id = 91;
    regular.panels = None;
    let mut other = regular.clone();
    other.tab.id = 55;
    // Identical locations must still have different numbers and activation targets.
    snapshot.workspaces.extend([regular, other]);
    let mut model = Model::new(vec![(WindowRef::capture(native.0).unwrap(), snapshot)]);
    model.toggle(0);
    assert_eq!(
        numbered_targets(&model),
        [
            (
                1,
                Target {
                    workspace: 7,
                    panel: Some((false, 11))
                }
            ),
            (
                1,
                Target {
                    workspace: 7,
                    panel: Some((true, 12))
                }
            ),
            (
                2,
                Target {
                    workspace: 91,
                    panel: None
                }
            ),
            (
                3,
                Target {
                    workspace: 55,
                    panel: None
                }
            ),
        ]
    );
    let heading = model
        .rows
        .iter()
        .find(|row| row.tab_number == Some(1))
        .unwrap();
    assert_eq!(heading.title, labels::tab_label(1));
    assert!(heading.icon.is_none());
    assert!(!heading.selectable());
    model.windows[0].1.workspaces.swap(0, 2);
    model.rebuild();
    assert_eq!(
        numbered_targets(&model),
        [
            (
                1,
                Target {
                    workspace: 55,
                    panel: None
                }
            ),
            (
                2,
                Target {
                    workspace: 91,
                    panel: None
                }
            ),
            (
                3,
                Target {
                    workspace: 7,
                    panel: Some((false, 11))
                }
            ),
            (
                3,
                Target {
                    workspace: 7,
                    panel: Some((true, 12))
                }
            ),
        ]
    );
    let heading = model
        .rows
        .iter()
        .find(|row| row.tab_number == Some(3))
        .unwrap();
    assert_eq!(heading.title, labels::tab_label(3));
    model.windows[0].1.workspaces.remove(1);
    model.rebuild();
    let renumbered: Vec<_> = numbered_targets(&model)
        .into_iter()
        .map(|(number, target)| (number, target.workspace))
        .collect();
    assert_eq!(renumbered, [(1, 55), (2, 7), (2, 7)]);
}

#[test]
fn tab_numbers_restart_for_each_instance_and_do_not_renumber_panel_tabs() {
    let first = NativeWindow::new();
    let second = NativeWindow::new();
    let mut model = Model::new(
        [&first, &second]
            .map(|window| (WindowRef::capture(window.0).unwrap(), snapshot()))
            .into(),
    );
    model.toggle(0);
    let second_index = model
        .rows
        .iter()
        .position(|row| row.instance().is_some_and(|window| window.hwnd == second.0))
        .unwrap();
    model.toggle(second_index);
    let headings: Vec<_> = model
        .rows
        .iter()
        .filter(|row| row.tab_number.is_some())
        .collect();
    assert_eq!(headings.len(), 2);
    assert!(headings
        .iter()
        .all(|row| row.tab_number == Some(1) && row.title == labels::tab_label(1)));
    for row in &model.rows {
        if matches!(
            row.action,
            Action::Command(Command::Activate(_, Target { panel: Some(_), .. }))
        ) {
            assert!(row.tab_number.is_none());
        }
    }
}

#[test]
fn a_single_normal_tab_gets_its_number_without_a_duplicate_menu_entry() {
    let native = NativeWindow::new();
    let mut snapshot = snapshot();
    snapshot.workspaces[0].panels = None;
    let model = Model::new(vec![(WindowRef::capture(native.0).unwrap(), snapshot)]);
    assert_eq!(model.rows.len(), 1);
    assert_eq!(model.rows[0].tab_number, Some(1));
    assert_eq!(model.rows[0].expanded, None);
    assert_eq!(model.rows[0].subtitle, "Example");
}
