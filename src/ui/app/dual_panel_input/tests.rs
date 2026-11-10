use super::consume_panel_tab;
use eframe::egui;

fn tab(modifiers: egui::Modifiers, pressed: bool) -> egui::Event {
    egui::Event::Key {
        key: egui::Key::Tab,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers,
    }
}

fn raw_input(events: Vec<egui::Event>) -> egui::RawInput {
    let mut raw = egui::RawInput {
        events,
        ..Default::default()
    };
    raw.viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .focused = Some(true);
    raw
}

#[test]
fn repeated_panel_switches_do_not_focus_other_controls() {
    let ctx = egui::Context::default();
    let mut right_panel = false;
    for expected_right in [true, false, true, false] {
        let _ = ctx.run_ui(raw_input(vec![tab(egui::Modifiers::NONE, true)]), |ui| {
            assert!(consume_panel_tab(ui.ctx(), true));
            right_panel = !right_panel;
            assert_eq!(right_panel, expected_right);
            assert!(ui.input(|input| input.events.is_empty()));
            let _ = ui.button("Toolbar action");
            let _ = ui.button("Other action");
            assert!(ui.memory(|memory| memory.focused().is_none()));
        });
        let _ = ctx.run_ui(raw_input(vec![tab(egui::Modifiers::NONE, false)]), |_| {});
    }
}

#[test]
fn panel_switch_recovers_from_non_text_widget_focus() {
    let ctx = egui::Context::default();
    let _ = ctx.run_ui(raw_input(vec![]), |ui| {
        ui.button("Toolbar action").request_focus();
    });
    let _ = ctx.run_ui(raw_input(vec![tab(egui::Modifiers::NONE, true)]), |ui| {
        assert!(ui.ctx().egui_wants_keyboard_input());
        assert!(!ui.ctx().text_edit_focused());
        assert!(consume_panel_tab(ui.ctx(), true));
        let _ = ui.button("Toolbar action");
        assert!(ui.memory(|memory| memory.focused().is_none()));
    });
}

#[test]
fn modified_tabs_remain_available_for_other_shortcuts() {
    for modifiers in [
        egui::Modifiers::CTRL,
        egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
        egui::Modifiers::SHIFT,
        egui::Modifiers::ALT,
        egui::Modifiers::COMMAND,
    ] {
        let ctx = egui::Context::default();
        let event = tab(modifiers, true);
        let _ = ctx.run_ui(raw_input(vec![event.clone()]), |ui| {
            assert!(!consume_panel_tab(ui.ctx(), true));
            assert_eq!(ui.input(|input| input.events.clone()), vec![event.clone()]);
        });
    }
}

#[test]
fn disabled_shortcut_preserves_normal_tab_traversal() {
    let ctx = egui::Context::default();
    let _ = ctx.run_ui(raw_input(vec![tab(egui::Modifiers::NONE, true)]), |ui| {
        assert!(!consume_panel_tab(ui.ctx(), false));
        assert_eq!(ui.input(|input| input.events.len()), 1);
        assert!(ui.button("Toolbar action").has_focus());
    });
}

#[test]
fn text_editing_keeps_tab_for_the_widget() {
    let ctx = egui::Context::default();
    let mut text = String::from("example");
    let field_id = egui::Id::new("panel_tab_text_field");
    let _ = ctx.run_ui(raw_input(vec![]), |ui| {
        ui.add(egui::TextEdit::singleline(&mut text).id(field_id))
            .request_focus();
    });
    let _ = ctx.run_ui(raw_input(vec![tab(egui::Modifiers::NONE, true)]), |ui| {
        assert!(ui.ctx().text_edit_focused());
        assert!(!consume_panel_tab(ui.ctx(), true));
        assert_eq!(ui.input(|input| input.events.len()), 1);
        ui.add(egui::TextEdit::singleline(&mut text).id(field_id));
    });
}

#[test]
fn modal_keeps_ownership_of_tab() {
    let ctx = egui::Context::default();
    let modal_id = egui::Id::new("panel_tab_modal");
    let _ = ctx.run_ui(raw_input(vec![]), |ui| {
        egui::Modal::new(modal_id).show(ui.ctx(), |ui| {
            let _ = ui.button("Confirm");
        });
    });
    let _ = ctx.run_ui(raw_input(vec![tab(egui::Modifiers::NONE, true)]), |ui| {
        assert!(ui.memory(|memory| memory.top_modal_layer().is_some()));
        assert!(!consume_panel_tab(ui.ctx(), true));
        assert_eq!(ui.input(|input| input.events.len()), 1);
    });
}
