use super::*;

struct Palette {
    ctx: egui::Context,
    state: CommandPaletteState,
    context: CommandContext,
    shortcuts: ShortcutBindings,
}

impl Palette {
    fn new() -> Self {
        let ctx = egui::Context::default();
        ctx.global_style_mut(|style| style.animation_time = 0.0);
        let mut state = CommandPaletteState::default();
        state.open(None);
        state.query = t!("command_palette.tabs").to_string();
        Self {
            ctx,
            state,
            context: CommandContext {
                can_add_tab: true,
                has_closed_tabs: true,
                ..Default::default()
            },
            shortcuts: ShortcutBindings::default(),
        }
    }

    fn frame(&mut self, events: Vec<egui::Event>) -> (Option<Command>, egui::FullOutput) {
        let mut command = None;
        let output = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 900.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                if let Some(executed) =
                    render_dialog(ui.ctx(), &mut self.state, &self.context, &self.shortcuts)
                {
                    command = Some(executed);
                }
            },
        );
        (command, output)
    }

    fn settle(&mut self) -> egui::FullOutput {
        for _ in 0..4 {
            self.frame(Vec::new());
        }
        self.frame(Vec::new()).1
    }

    fn assert_help(&self, output: &egui::FullOutput, command: Command) {
        let help = input_hints::help(command, &self.shortcuts);
        assert!(has_text(output, &help), "must describe {command:?}");
        let highlights = output
            .shapes
            .iter()
            .filter(|shape| {
                matches!(&shape.shape,
            egui::Shape::Rect(rect) if rect.fill.a() > 0
                && (rect.rect.height() - 32.0).abs() < 1.0
                && rect.rect.width() > 600.0)
            })
            .count();
        assert_eq!(
            highlights, 1,
            "only one row may have the selection highlight"
        );
    }
}

fn has_text(output: &egui::FullOutput, expected: &str) -> bool {
    output.shapes.iter().any(|shape| {
        matches!(&shape.shape,
        egui::Shape::Text(text) if text.galley.text() == expected)
    })
}

fn row_position(output: &egui::FullOutput, command: Command) -> egui::Pos2 {
    let label = t!(command.label_key());
    output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == label.as_ref() => {
                Some(text.pos + egui::vec2(8.0, 5.0))
            }
            _ => None,
        })
        .expect("command row must be rendered")
}

#[test]
fn mouse_and_keyboard_share_one_selection_and_stationary_mouse_does_not_override_keys() {
    let mut palette = Palette::new();
    let output = palette.settle();
    let pointer = row_position(&output, Command::CloseTab);
    let (_, output) = palette.frame(vec![egui::Event::PointerMoved(pointer)]);
    assert_eq!(palette.state.selected, 1);
    palette.assert_help(&output, Command::CloseTab);
    assert!(!has_text(
        &output,
        &input_hints::help(Command::NewTab, &palette.shortcuts)
    ));

    let (_, output) = palette.frame(vec![super::tests::key(egui::Key::ArrowDown)]);
    assert_eq!(palette.state.selected, 2);
    palette.assert_help(&output, Command::ReopenTab);
    let (_, output) = palette.frame(Vec::new());
    assert_eq!(palette.state.selected, 2);
    palette.assert_help(&output, Command::ReopenTab);

    let (_, output) = palette.frame(vec![egui::Event::PointerMoved(
        pointer + egui::vec2(1.0, 0.0),
    )]);
    assert_eq!(palette.state.selected, 1);
    palette.assert_help(&output, Command::CloseTab);
    assert_eq!(
        palette.frame(vec![super::tests::key(egui::Key::Enter)]).0,
        Some(Command::CloseTab)
    );
}

#[test]
fn disabled_commands_follow_mouse_and_show_help_and_reason_without_execution() {
    let mut palette = Palette::new();
    palette.context.has_closed_tabs = false;
    let output = palette.settle();
    let pointer = row_position(&output, Command::ReopenTab);
    let (_, output) = palette.frame(vec![egui::Event::PointerMoved(pointer)]);
    assert_eq!(palette.state.selected, 2);
    palette.assert_help(&output, Command::ReopenTab);
    assert!(has_text(&output, &t!("command_palette.no_closed_tabs")));
    assert_eq!(
        palette.frame(vec![super::tests::key(egui::Key::Enter)]).0,
        None
    );
    assert!(palette.state.active);
    for pressed in [true, false] {
        assert_eq!(
            palette
                .frame(vec![egui::Event::PointerButton {
                    pos: pointer,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }])
                .0,
            None
        );
    }
    assert!(palette.state.active);
}

#[test]
fn clicking_a_different_row_executes_that_row_and_closes() {
    let mut palette = Palette::new();
    let output = palette.settle();
    let pointer = row_position(&output, Command::CloseTab);
    palette.frame(vec![
        egui::Event::PointerMoved(pointer),
        egui::Event::PointerButton {
            pos: pointer,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        },
    ]);
    assert_eq!(
        palette
            .frame(vec![egui::Event::PointerButton {
                pos: pointer,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }])
            .0,
        Some(Command::CloseTab)
    );
    assert!(!palette.state.active);
}

#[test]
fn editing_the_query_resets_selection_without_stationary_pointer_interference() {
    let mut palette = Palette::new();
    let output = palette.settle();
    let pointer = row_position(&output, Command::ReopenTab);
    palette.frame(vec![egui::Event::PointerMoved(pointer)]);
    assert_eq!(palette.state.selected, 2);
    // The query keeps keyboard focus even when a disabled or enabled row is hovered.
    assert_eq!(
        palette.ctx.memory(|memory| memory.focused()),
        Some(input_id())
    );
    let (_, output) = palette.frame(vec![egui::Event::Text(" no matching command".into())]);
    assert_eq!(palette.state.selected, 0);
    assert!(has_text(&output, &t!("command_palette.no_matches")));
    assert!(!has_text(
        &output,
        &input_hints::help(Command::ReopenTab, &palette.shortcuts)
    ));
    assert_eq!(
        palette.frame(vec![super::tests::key(egui::Key::Enter)]).0,
        None
    );
    assert!(palette.state.active);
}
