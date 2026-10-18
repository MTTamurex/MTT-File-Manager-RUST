use super::*;
use egui::accesskit::{Action, Role, Toggled};

fn selected_control_rect(output: &egui::FullOutput, height: f32) -> egui::epaint::RectShape {
    output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Rect(rect)
                if (rect.rect.height() - height).abs() < 0.1
                    && rect.fill != Color32::TRANSPARENT =>
            {
                Some(rect.clone())
            }
            _ => None,
        })
        .expect("selected control should have a filled rectangle")
}

#[test]
fn segmented_selections_match_the_appearance_selector_colors_in_every_theme() {
    use crate::app::navigation_state::ThemeMode;
    for mode in [
        ThemeMode::Light,
        ThemeMode::Dark,
        ThemeMode::DarkDeep,
        ThemeMode::GitHubDark,
    ] {
        let ctx = egui::Context::default();
        theme::apply_theme_visuals(&ctx, mode);
        let segments = ctx.run_ui(egui::RawInput::default(), |ui| {
            segmented_choice(ui, &["First", "Second"], 1);
        });
        let choices = ctx.run_ui(egui::RawInput::default(), |ui| {
            choice_list(ui, &["First", "Second"], 1);
        });
        let segment = selected_control_rect(&segments, SEGMENT_HEIGHT);
        let choice = selected_control_rect(&choices, OPTION_ROW_HEIGHT);
        // Choice rows paint their fill and border as separate shapes.
        let choice_border = choices
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Rect(rect) if rect.rect == choice.rect && rect.stroke.width > 0.0 => {
                    Some(rect.stroke)
                }
                _ => None,
            })
            .expect("selected appearance option should have a border");
        assert_eq!(segment.fill, choice.fill, "{mode:?}");
        assert_eq!(segment.stroke, choice_border, "{mode:?}");
        assert_eq!(segment.fill, accent_tint(&ctx));
        assert_eq!(segment.stroke, Stroke::new(1.0, theme::accent_color(&ctx)));
    }
}

#[test]
fn light_segmented_selection_uses_the_light_appearance_blue() {
    let ctx = egui::Context::default();
    theme::apply_theme_visuals(&ctx, crate::app::navigation_state::ThemeMode::Light);
    let output = ctx.run_ui(egui::RawInput::default(), |ui| {
        segmented_choice(ui, &["First", "Second"], 1);
    });
    let selected = selected_control_rect(&output, SEGMENT_HEIGHT);
    assert_eq!(
        selected.fill,
        Color32::from_rgba_unmultiplied(0, 120, 215, 24)
    );
    assert_eq!(
        selected.stroke,
        Stroke::new(1.0, Color32::from_rgb(0, 120, 215))
    );
}

fn key_input(key: egui::Key) -> egui::RawInput {
    let mut input = egui::RawInput::default();
    input.events.push(egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    input
}

fn repeated_key_input(key: egui::Key) -> egui::RawInput {
    let mut input = key_input(key);
    input.events.push(egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: true,
        modifiers: egui::Modifiers::NONE,
    });
    input
}

fn keyboard_activates(
    key: egui::Key,
    mut render: impl FnMut(&mut egui::Ui) -> (egui::Id, bool),
) -> bool {
    let ctx = egui::Context::default();
    let id = std::cell::Cell::new(egui::Id::NULL);
    let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
        let (widget_id, _) = render(ui);
        id.set(widget_id);
    });
    ctx.memory_mut(|memory| memory.request_focus(id.get()));

    let activated = std::cell::Cell::new(false);
    let _ = ctx.run_ui(key_input(key), |ui| {
        let (_, was_activated) = render(ui);
        activated.set(was_activated);
    });
    activated.get()
}

#[test]
fn custom_controls_are_keyboard_operable() {
    assert!(keyboard_activates(egui::Key::Enter, |ui| {
        let id = ui.next_auto_id();
        (id, nav_item(ui, "Navigation", false))
    }));

    assert!(keyboard_activates(egui::Key::Enter, |ui| {
        let group_id = ui.next_auto_id();
        (
            group_id.with(0),
            segmented_choice(ui, &["First", "Second"], 1) == Some(0),
        )
    }));

    assert!(keyboard_activates(egui::Key::Enter, |ui| {
        let id = ui.next_auto_id();
        (id, choice_list(ui, &["First", "Second"], 1) == Some(0))
    }));

    let mut row_value = false;
    assert!(keyboard_activates(egui::Key::Space, |ui| {
        let id = ui.next_auto_id();
        (id, toggle_row(ui, "Row toggle", &mut row_value))
    }));

    let mut switch_value = false;
    assert!(keyboard_activates(egui::Key::Space, |ui| {
        let response = toggle_switch(ui, &mut switch_value, "Rule enabled");
        (response.id, response.changed())
    }));
}

#[test]
fn segmented_choice_arrows_move_once_and_move_focus() {
    let ctx = egui::Context::default();
    let group_id = std::cell::Cell::new(egui::Id::NULL);
    let mut selected = 0;
    let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
        group_id.set(ui.next_auto_id());
        segmented_choice(ui, &["First", "Second", "Third"], selected);
    });
    ctx.memory_mut(|memory| memory.request_focus(group_id.get().with(0)));

    let first_move = std::cell::Cell::new(None);
    let _ = ctx.run_ui(repeated_key_input(egui::Key::ArrowRight), |ui| {
        first_move.set(segmented_choice(
            ui,
            &["First", "Second", "Third"],
            selected,
        ));
    });
    assert_eq!(first_move.get(), Some(1));
    selected = 1;

    let second_move = std::cell::Cell::new(None);
    let _ = ctx.run_ui(key_input(egui::Key::ArrowLeft), |ui| {
        second_move.set(segmented_choice(
            ui,
            &["First", "Second", "Third"],
            selected,
        ));
    });
    assert_eq!(second_move.get(), Some(0));
}

#[test]
fn choice_list_arrows_move_once_and_support_both_orientations() {
    let ctx = egui::Context::default();
    let first_id = std::cell::Cell::new(egui::Id::NULL);
    let mut selected = 0;
    let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
        first_id.set(ui.next_auto_id());
        choice_list(ui, &["First", "Second", "Third"], selected);
    });
    ctx.memory_mut(|memory| memory.request_focus(first_id.get()));

    let first_move = std::cell::Cell::new(None);
    let _ = ctx.run_ui(key_input(egui::Key::ArrowDown), |ui| {
        first_move.set(choice_list(ui, &["First", "Second", "Third"], selected));
    });
    assert_eq!(first_move.get(), Some(1));
    selected = 1;

    let second_move = std::cell::Cell::new(None);
    let _ = ctx.run_ui(key_input(egui::Key::ArrowRight), |ui| {
        second_move.set(choice_list(ui, &["First", "Second", "Third"], selected));
    });
    assert_eq!(second_move.get(), Some(2));
}

#[test]
fn custom_controls_expose_accessibility_semantics() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut row_value = true;
    let mut switch_value = false;
    let output = ctx.run_ui(egui::RawInput::default(), |ui| {
        ui.set_width(400.0);
        nav_item(ui, "Navigation", true);
        segmented_choice(ui, &["Segment one", "Segment two"], 0);
        choice_list(ui, &["Choice one", "Choice two"], 1);
        toggle_row(ui, "Row toggle", &mut row_value);
        toggle_switch(ui, &mut switch_value, "Images enabled");
    });
    let nodes = &output
        .platform_output
        .accesskit_update
        .expect("accessibility tree")
        .nodes;

    let assert_node = |role, label: &str, toggled| {
        let node = nodes
            .iter()
            .map(|(_, node)| node)
            .find(|node| node.role() == role && node.label() == Some(label))
            .unwrap_or_else(|| panic!("missing accessibility node: {label}"));
        assert_eq!(node.toggled(), Some(toggled));
        assert!(node.supports_action(Action::Focus));
        assert!(node.supports_action(Action::Click));
    };

    assert_node(Role::Button, "Navigation", Toggled::True);
    assert_node(Role::RadioButton, "Segment one", Toggled::True);
    assert_node(Role::RadioButton, "Segment two", Toggled::False);
    assert_node(Role::RadioButton, "Choice one", Toggled::False);
    assert_node(Role::RadioButton, "Choice two", Toggled::True);
    assert_node(Role::CheckBox, "Row toggle", Toggled::True);
    assert_node(Role::CheckBox, "Images enabled", Toggled::False);
}
