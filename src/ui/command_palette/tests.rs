use super::*;

pub(super) fn key(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

#[test]
fn escape_consumes_input_and_restores_focus() {
    let ctx = egui::Context::default();
    let previous = egui::Id::new("previous_focus");
    let mut state = CommandPaletteState::default();
    state.open(Some(previous));
    let _ = ctx.run_ui(
        egui::RawInput {
            events: vec![key(egui::Key::Escape)],
            ..Default::default()
        },
        |ui| {
            assert!(render_dialog(
                ui.ctx(),
                &mut state,
                &CommandContext::default(),
                &ShortcutBindings::default()
            )
            .is_none());
            assert!(!state.active);
            assert_eq!(ui.ctx().memory(|memory| memory.focused()), Some(previous));
            block_background_input(ui.ctx());
            assert!(!ui.ctx().input(|input| input.key_pressed(egui::Key::Escape)));
        },
    );
}

#[test]
fn disabled_enter_does_not_execute_and_background_shortcuts_are_blocked() {
    let ctx = egui::Context::default();
    let mut state = CommandPaletteState::default();
    state.open(None);
    let _ =
        ctx.run_ui(
            egui::RawInput {
                events: vec![key(egui::Key::Enter), key(egui::Key::Delete)],
                ..Default::default()
            },
            |ui| {
                // The first command is NewTab, disabled when no more tabs can be added.
                assert!(render_dialog(
                    ui.ctx(),
                    &mut state,
                    &CommandContext::default(),
                    &ShortcutBindings::default()
                )
                .is_none());
                assert!(state.active);
                block_background_input(ui.ctx());
                assert!(!ui.ctx().input(|input| input.key_pressed(egui::Key::Enter)
                    || input.key_pressed(egui::Key::Delete)));
            },
        );
}

#[test]
fn enabled_enter_returns_one_command_and_closes() {
    let ctx = egui::Context::default();
    let mut state = CommandPaletteState::default();
    state.open(None);
    let context = CommandContext {
        can_add_tab: true,
        ..Default::default()
    };
    let _ = ctx.run_ui(
        egui::RawInput {
            events: vec![key(egui::Key::Enter)],
            ..Default::default()
        },
        |ui| {
            assert_eq!(
                render_dialog(ui.ctx(), &mut state, &context, &ShortcutBindings::default()),
                Some(Command::NewTab)
            );
            assert!(!state.active);
            block_background_input(ui.ctx());
            assert!(!ui.ctx().input(|input| input.key_pressed(egui::Key::Enter)));
        },
    );
}

#[test]
fn holding_palette_shortcut_does_not_toggle_repeatedly() {
    let ctx = egui::Context::default();
    for repeat in [false, true] {
        let _ = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::P,
                    physical_key: None,
                    pressed: true,
                    repeat,
                    modifiers: egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
                }],
                ..Default::default()
            },
            |ui| {
                assert_eq!(
                    consume_toggle(&ShortcutBindings::default(), ui.ctx()),
                    !repeat
                );
                assert!(!consume_toggle(&ShortcutBindings::default(), ui.ctx()));
            },
        );
    }
}

#[test]
fn background_blocker_discards_text_clipboard_scroll_and_external_drops() {
    let ctx = egui::Context::default();
    let _ = ctx.run_ui(
        egui::RawInput {
            events: vec![
                egui::Event::Text("query".into()),
                egui::Event::Paste("text".into()),
                egui::Event::Cut,
                egui::Event::Copy,
                egui::Event::MouseWheel {
                    phase: egui::TouchPhase::Move,
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, 20.0),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            dropped_files: vec![egui::DroppedFile {
                path: Some(std::path::PathBuf::from("file.txt")),
                ..Default::default()
            }],
            ..Default::default()
        },
        |ui| {
            block_background_input(ui.ctx());
            ui.ctx().input(|input| {
                assert!(input.events.is_empty());
                assert!(input.raw.dropped_files.is_empty());
                assert_eq!(input.smooth_scroll_delta, egui::Vec2::ZERO);
            });
        },
    );
}

#[test]
fn help_does_not_move_the_palette_when_selection_changes() {
    let ctx = egui::Context::default();
    ctx.global_style_mut(|style| style.animation_time = 0.0);
    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 900.0));
    let mut state = CommandPaletteState::default();
    state.open(None);
    let shortcuts = ShortcutBindings::default();
    let mut rectangles = Vec::new();
    for selected in [0, crate::app::command_palette::catalog::COMMANDS.len() - 2] {
        state.selected = selected;
        // Let modal measurement and both scroll areas settle before comparing layouts.
        for _ in 0..6 {
            let _ = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(viewport),
                    ..Default::default()
                },
                |ui| {
                    assert!(render_dialog(
                        ui.ctx(),
                        &mut state,
                        &CommandContext::default(),
                        &shortcuts
                    )
                    .is_none());
                },
            );
        }
        rectangles.push(
            ctx.memory(|memory| memory.area_rect(egui::Id::new("command_palette_modal")))
                .unwrap(),
        );
    }
    assert!(
        (rectangles[0].height() - rectangles[1].height()).abs() < 1.0,
        "{rectangles:?}"
    );
    assert!(
        (rectangles[0].center().y - rectangles[1].center().y).abs() < 1.0,
        "{rectangles:?}"
    );
}

#[test]
fn space_types_in_the_query_instead_of_executing_selected_preview() {
    let ctx = egui::Context::default();
    let mut state = CommandPaletteState::default();
    state.open(None);
    let context = CommandContext {
        has_selection: true,
        can_preview_selected: true,
        ..Default::default()
    };
    for _ in 0..4 {
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            let _ = render_dialog(ui.ctx(), &mut state, &context, &ShortcutBindings::default());
        });
    }
    state.selected = crate::app::command_palette::catalog::COMMANDS
        .iter()
        .position(|command| *command == Command::PreviewSelected)
        .unwrap();
    let _ = ctx.run_ui(
        egui::RawInput {
            events: vec![key(egui::Key::Space), egui::Event::Text(" ".into())],
            ..Default::default()
        },
        |ui| {
            assert_eq!(
                render_dialog(ui.ctx(), &mut state, &context, &ShortcutBindings::default()),
                None
            );
            assert_eq!(state.query, " ");
            assert!(state.active);
            block_background_input(ui.ctx());
            assert!(!ui.input(|input| input.key_pressed(egui::Key::Space)));
        },
    );
}

#[test]
fn new_file_commands_execute_only_when_enabled_and_explicitly_confirmed() {
    let shortcuts = ShortcutBindings::default();
    for command in [
        Command::PreviewSelected,
        Command::Copy,
        Command::Cut,
        Command::Paste,
        Command::Delete,
    ] {
        for enabled in [true, false] {
            let ctx = egui::Context::default();
            let mut state = CommandPaletteState::default();
            state.open(None);
            state.selected = crate::app::command_palette::catalog::COMMANDS
                .iter()
                .position(|candidate| *candidate == command)
                .unwrap();
            let context = CommandContext {
                has_selection: enabled,
                can_preview_selected: enabled,
                can_copy: enabled,
                can_cut: enabled,
                has_clipboard_files: enabled,
                can_paste: enabled,
                can_delete: enabled,
                ..Default::default()
            };
            for _ in 0..4 {
                let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                    assert_eq!(
                        render_dialog(ui.ctx(), &mut state, &context, &shortcuts),
                        None
                    );
                });
            }
            let _ = ctx.run_ui(
                egui::RawInput {
                    events: vec![key(egui::Key::Enter)],
                    ..Default::default()
                },
                |ui| {
                    assert_eq!(
                        render_dialog(ui.ctx(), &mut state, &context, &shortcuts),
                        enabled.then_some(command)
                    );
                    assert_eq!(state.active, !enabled);
                },
            );
        }
    }
}

#[test]
fn clipboard_and_delete_events_in_search_do_not_execute_file_commands() {
    for event in [
        egui::Event::Copy,
        egui::Event::Cut,
        egui::Event::Paste("filter text".into()),
        key(egui::Key::Delete),
    ] {
        let ctx = egui::Context::default();
        let mut state = CommandPaletteState::default();
        state.open(None);
        let context = CommandContext {
            has_selection: true,
            can_copy: true,
            can_cut: true,
            has_clipboard_files: true,
            can_paste: true,
            can_delete: true,
            ..Default::default()
        };
        for _ in 0..4 {
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                let _ = render_dialog(ui.ctx(), &mut state, &context, &ShortcutBindings::default());
            });
        }
        state.selected = crate::app::command_palette::catalog::COMMANDS
            .iter()
            .position(|command| *command == Command::Delete)
            .unwrap();
        let _ = ctx.run_ui(
            egui::RawInput {
                events: vec![event.clone()],
                ..Default::default()
            },
            |ui| {
                assert_eq!(
                    render_dialog(ui.ctx(), &mut state, &context, &ShortcutBindings::default()),
                    None
                );
                assert!(state.active);
                if matches!(event, egui::Event::Paste(_)) {
                    assert_eq!(state.query, "filter text");
                }
                block_background_input(ui.ctx());
                assert!(ui.input(|input| input.events.is_empty()));
            },
        );
    }
}

#[test]
fn disk_analyzer_command_obeys_eligibility_before_accepting_enter() {
    for drive in [None, Some('D')] {
        let ctx = egui::Context::default();
        let mut state = CommandPaletteState::default();
        state.open(None);
        state.query = t!(Command::DiskAnalysis.label_key()).to_string();
        let context = CommandContext {
            disk_analysis_drive: drive,
            ..Default::default()
        };
        let shortcuts = ShortcutBindings::default();
        for _ in 0..4 {
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                assert_eq!(
                    render_dialog(ui.ctx(), &mut state, &context, &shortcuts),
                    None
                );
            });
        }
        let _ = ctx.run_ui(
            egui::RawInput {
                events: vec![key(egui::Key::Enter)],
                ..Default::default()
            },
            |ui| {
                assert_eq!(
                    render_dialog(ui.ctx(), &mut state, &context, &shortcuts),
                    drive.map(|_| Command::DiskAnalysis)
                );
                assert_eq!(state.active, drive.is_none());
            },
        );
    }
}
