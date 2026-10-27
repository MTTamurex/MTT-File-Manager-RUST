use super::*;
use egui::{Event, PointerButton, Pos2, RawInput, Rect, Vec2};

#[test]
fn network_server_breadcrumb_preserves_the_unc_navigation_target() {
    for path in [r"\\server", r"\\server\"] {
        assert_eq!(
            breadcrumb_segments(path, &[]),
            vec![("server".into(), path.into())]
        );
    }
}

struct ToolbarTestState {
    ctx: egui::Context,
    path_input: String,
    is_editing_path: bool,
    show_address_history_menu: bool,
    address_bar_focus_request: bool,
    search_query: String,
    recent_paths: Vec<(String, String)>,
    svg_manager: SvgIconManager,
    copied_text: Option<String>,
}

impl Default for ToolbarTestState {
    fn default() -> Self {
        Self {
            ctx: egui::Context::default(),
            path_input: String::new(),
            is_editing_path: false,
            show_address_history_menu: false,
            address_bar_focus_request: false,
            search_query: String::new(),
            recent_paths: vec![("Folder".to_owned(), "C:\\Folder".to_owned())],
            svg_manager: SvgIconManager::default(),
            copied_text: None,
        }
    }
}

impl ToolbarTestState {
    fn render(&mut self, events: Vec<Event>) -> Option<ToolbarAction> {
        let Self {
            ctx,
            path_input,
            is_editing_path,
            show_address_history_menu,
            address_bar_focus_request,
            search_query,
            recent_paths,
            svg_manager,
            copied_text,
        } = self;
        let mut action = None;
        let output = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 120.0))),
                events,
                ..RawInput::default()
            },
            |ui| {
                let mut thumbnail_size = 100.0;
                action = render_toolbar(
                    ui,
                    "C:\\",
                    &[],
                    None,
                    path_input,
                    is_editing_path,
                    show_address_history_menu,
                    address_bar_focus_request,
                    search_query,
                    recent_paths,
                    false,
                    false,
                    true,
                    ViewMode::Grid,
                    SortMode::Name,
                    false,
                    &mut thumbnail_size,
                    false,
                    false,
                    None,
                    svg_manager,
                    &crate::app::shortcuts::ShortcutBindings::default(),
                );
            },
        );
        *copied_text = output.platform_output.commands.iter().find_map(|command| {
            if let egui::OutputCommand::CopyText(text) = command {
                Some(text.clone())
            } else {
                None
            }
        });
        action
    }

    fn selection(&self) -> egui::text::CCursorRange {
        let id = self.ctx.memory(|memory| memory.focused()).unwrap();
        egui::TextEdit::load_state(&self.ctx, id)
            .unwrap()
            .cursor
            .char_range()
            .unwrap()
    }
}

#[test]
fn clicking_empty_address_area_defers_focus_until_edit_field_is_rendered() {
    let mut state = ToolbarTestState::default();
    let click_pos = Pos2::new(500.0, 13.0);
    state.render(Vec::new());
    state.render(vec![Event::PointerMoved(click_pos)]);
    state.render(vec![Event::PointerButton {
        pos: click_pos,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    }]);
    let action = state.render(vec![Event::PointerButton {
        pos: click_pos,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    assert!(matches!(
        action,
        Some(ToolbarAction::StartAddressEditWithHistory)
    ));
    assert!(state.address_bar_focus_request);
    state.path_input = crate::ui::address_bar::editable_path("C:\\", None);
    state.is_editing_path = true;
    state.show_address_history_menu = true;
    state.render(Vec::new());
    assert!(!state.address_bar_focus_request);
    assert!(state.is_editing_path);
    assert!(state.show_address_history_menu);
    state.render(vec![Event::Text("x".to_owned())]);
    assert_eq!(state.path_input, "x");
    assert!(state.show_address_history_menu);
}

#[test]
fn explicit_focus_selects_unicode_path_and_copy_preserves_the_full_path() {
    let mut state = ToolbarTestState {
        path_input: "C:\\Folder\\Pasta de teste 日本語".into(),
        is_editing_path: true,
        address_bar_focus_request: true,
        ..Default::default()
    };
    state.render(Vec::new());
    assert_eq!(
        state.selection().as_sorted_char_range(),
        0.into()..state.path_input.chars().count().into()
    );
    state.render(vec![Event::Copy]);
    assert_eq!(
        state.copied_text.as_deref(),
        Some(state.path_input.as_str())
    );

    // Moving the caret must not cause the next frame to reselect everything.
    state.render(vec![Event::Key {
        key: egui::Key::ArrowLeft,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    assert!(state.selection().is_empty());

    // Ctrl+L issues this same request even while the field is already editing.
    state.address_bar_focus_request = true;
    state.render(Vec::new());
    assert_eq!(
        state.selection().as_sorted_char_range(),
        0.into()..state.path_input.chars().count().into()
    );
    state.render(vec![Event::Copy]);
    assert_eq!(
        state.copied_text.as_deref(),
        Some(state.path_input.as_str())
    );
}
