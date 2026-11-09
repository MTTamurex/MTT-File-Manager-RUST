use super::*;
use mtt_search_protocol::SearchResultItem;

fn state() -> GlobalSearchState {
    let (tx, _) = std::sync::mpsc::channel();
    let (_, rx) = std::sync::mpsc::channel();
    let mut state = GlobalSearchState::new(tx, rx);
    state.results = [
        r"C:\Folder\file.txt",
        r"D:\Other\file.txt",
        r"C:\Folder\hidden.txt",
    ]
    .into_iter()
    .map(|path| SearchResultItem {
        name: "file.txt".into(),
        full_path: path.into(),
        is_dir: false,
        size: 1,
    })
    .collect();
    state.selected_indices.extend([0, 1, 2]);
    state
}

fn frame(
    modifiers: egui::Modifiers,
    events: Vec<egui::Event>,
    mut check: impl FnMut(&egui::Context),
) {
    let ctx = egui::Context::default();
    let _ = ctx.run_ui(
        egui::RawInput {
            modifiers,
            events,
            ..Default::default()
        },
        |ui| check(ui.ctx()),
    );
}

#[test]
fn search_batch_delete_keeps_origins_and_excludes_hidden_selection() {
    for modifiers in [egui::Modifiers::NONE, egui::Modifiers::SHIFT] {
        let mut state = state();
        frame(
            modifiers,
            vec![egui::Event::Key {
                key: egui::Key::Delete,
                physical_key: Some(egui::Key::Delete),
                pressed: true,
                repeat: false,
                modifiers,
            }],
            |ctx| {
                let action =
                    selection_action(&mut state, &ShortcutBindings::default(), ctx, &[1, 0]);
                let Some(ResultAction::Delete(paths, permanently)) = action else {
                    panic!("expected delete");
                };
                assert_eq!(permanently, modifiers.shift);
                assert_eq!(
                    paths,
                    [
                        PathBuf::from(r"D:\Other\file.txt"),
                        PathBuf::from(r"C:\Folder\file.txt")
                    ]
                );
                assert!(
                    selection_action(&mut state, &ShortcutBindings::default(), ctx, &[1, 0])
                        .is_none()
                );
            },
        );
    }
}

#[test]
fn search_batch_copy_and_cut_include_multiple_origins() {
    for cut in [false, true] {
        let mut state = state();
        frame(
            egui::Modifiers::CTRL,
            vec![if cut {
                egui::Event::Cut
            } else {
                egui::Event::Copy
            }],
            |ctx| {
                let action =
                    selection_action(&mut state, &ShortcutBindings::default(), ctx, &[0, 1]);
                let paths = match (cut, action) {
                    (true, Some(ResultAction::Cut(paths)))
                    | (false, Some(ResultAction::Copy(paths))) => paths,
                    _ => panic!("expected clipboard operation"),
                };
                assert_eq!(paths.len(), 2);
                assert_ne!(paths[0].parent(), paths[1].parent());
            },
        );
    }
}

#[test]
fn search_delete_never_falls_back_to_first_result_without_selection() {
    let mut state = state();
    state.selected_indices.clear();
    frame(egui::Modifiers::SHIFT, vec![egui::Event::Cut], |ctx| {
        assert!(selection_action(&mut state, &ShortcutBindings::default(), ctx, &[0, 1]).is_none());
    });
}

#[test]
fn search_shift_delete_native_event_does_not_cut_files() {
    let mut state = state();
    frame(egui::Modifiers::SHIFT, vec![egui::Event::Cut], |ctx| {
        assert!(matches!(
            selection_action(&mut state, &ShortcutBindings::default(), ctx, &[0, 1]),
            Some(ResultAction::Delete(_, true))
        ));
    });
}

#[test]
fn search_file_shortcuts_do_not_escape_focused_filter_text_fields() {
    let mut state = state();
    let mut text = String::new();
    let ctx = egui::Context::default();
    let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
        ui.text_edit_singleline(&mut text).request_focus();
    });
    let _ = ctx.run_ui(
        egui::RawInput {
            events: vec![egui::Event::Key {
                key: egui::Key::Delete,
                physical_key: Some(egui::Key::Delete),
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        },
        |ui| {
            assert!(
                selection_action(&mut state, &ShortcutBindings::default(), ui.ctx(), &[0, 1])
                    .is_none()
            );
        },
    );
}
