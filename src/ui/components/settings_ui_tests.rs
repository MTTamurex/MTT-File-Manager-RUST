use super::*;
use egui::accesskit::{Action, Role, Toggled};

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
