use super::context_menu::{close_control, tab_context_menu};
use crate::tabs::TabAction;
use eframe::egui::{self, pos2, vec2, Context, Event, Modifiers, PointerButton, RawInput, Sense};

fn pointer_input(pos: egui::Pos2, button: PointerButton, pressed: bool) -> RawInput {
    RawInput {
        events: vec![
            Event::PointerMoved(pos),
            Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers: Modifiers::NONE,
            },
        ],
        ..Default::default()
    }
}

#[test]
fn tab_context_menu_duplicates_the_clicked_tab_without_selecting_it_first() {
    click_command_without_selecting_tab(None, TabAction::Duplicate, "tabs.duplicate", false);
}

#[test]
fn right_click_on_pin_keeps_the_tab_context_menu_available() {
    click_command_without_selecting_tab(None, TabAction::TogglePin, "tabs.unpin", true);
}

#[test]
fn context_menu_reopens_a_closed_tab_without_selecting_the_clicked_tab() {
    click_command_without_selecting_tab(None, TabAction::ReopenClosed, "tabs.reopen_closed", false);
}

#[test]
fn clicking_empty_title_strip_dismisses_tab_menu_without_opening_a_tab_or_dragging() {
    use crate::infrastructure::windows::window_subclass::is_caption_drag_region_active;

    let ctx = Context::default();
    let mut new_tab_clicked = false;
    let mut action = None;
    let mut render = |ui: &mut egui::Ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let response = ui.allocate_response(vec2(180.0, 36.0), Sense::click());
            action = action.or(tab_context_menu(
                &response,
                false,
                true,
                false,
                false,
                None,
                None,
                Some(false),
            ));
            new_tab_clicked |= super::new_tab_area::render_new_tab_and_drag_area(
                ui,
                &ctx,
                36.0,
                138.0,
                egui::Color32::GRAY,
                egui::Color32::LIGHT_GRAY,
                egui::Color32::WHITE,
                false,
            );
        });
    };
    let _ = ctx.run_ui(RawInput::default(), &mut render);
    assert!(is_caption_drag_region_active());
    for pressed in [true, false] {
        let _ = ctx.run_ui(
            pointer_input(pos2(40.0, 20.0), PointerButton::Secondary, pressed),
            &mut render,
        );
    }
    let _ = ctx.run_ui(RawInput::default(), &mut render);
    assert!(egui::Popup::is_any_open(&ctx));
    assert!(!is_caption_drag_region_active());

    for pressed in [true, false] {
        let output = ctx.run_ui(
            pointer_input(pos2(500.0, 20.0), PointerButton::Primary, pressed),
            &mut render,
        );
        assert!(output.viewport_output.values().all(|viewport| !viewport
            .commands
            .iter()
            .any(|command| matches!(command, egui::ViewportCommand::StartDrag))));
        if pressed {
            assert!(!is_caption_drag_region_active());
        }
    }
    assert!(!egui::Popup::is_any_open(&ctx));
    assert!(is_caption_drag_region_active());
    assert!(!new_tab_clicked);
    assert_eq!(action, None);
}

#[test]
fn reopen_menu_entry_is_disabled_without_history_and_hidden_for_pane_tabs() {
    for availability in [Some(false), None] {
        let ctx = Context::default();
        let mut action = None;
        let mut render = |ui: &mut egui::Ui| {
            let response = ui.allocate_response(vec2(180.0, 36.0), Sense::click());
            action = action.or(tab_context_menu(
                &response,
                false,
                true,
                false,
                false,
                None,
                None,
                availability,
            ));
        };
        let _ = ctx.run_ui(RawInput::default(), &mut render);
        for pressed in [true, false] {
            let _ = ctx.run_ui(
                pointer_input(pos2(40.0, 20.0), PointerButton::Secondary, pressed),
                &mut render,
            );
        }
        let output = ctx.run_ui(RawInput::default(), &mut render);
        let position = output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::epaint::Shape::Text(text)
                if text.galley.text() == rust_i18n::t!("tabs.reopen_closed") =>
            {
                Some(shape.shape.visual_bounding_rect().center())
            }
            _ => None,
        });
        assert_eq!(position.is_some(), availability.is_some());
        if let Some(position) = position {
            for pressed in [true, false] {
                let _ = ctx.run_ui(
                    pointer_input(position, PointerButton::Primary, pressed),
                    &mut render,
                );
            }
        }
        assert_eq!(action, None);
    }
}

#[test]
fn quick_access_commands_do_not_select_or_pin_the_tab() {
    for (command, label) in [
        (TabAction::PinQuickAccess, "context_menu.pin_quick_access"),
        (
            TabAction::UnpinQuickAccess,
            "context_menu.unpin_quick_access",
        ),
    ] {
        click_command_without_selecting_tab(Some(command), command, label, false);
    }
}

#[test]
fn folder_tree_commands_do_not_select_the_clicked_tab() {
    for (action, label) in [
        (TabAction::ExpandFolderTree, "tabs.expand_folder_tree"),
        (TabAction::CollapseFolderTree, "tabs.collapse_folder_tree"),
    ] {
        click_command_without_selecting_tab(None, action, label, false);
    }
}

fn click_command_without_selecting_tab(
    quick_access: Option<TabAction>,
    expected: TabAction,
    label: &str,
    pinned: bool,
) {
    let ctx = Context::default();
    let mut action = None;
    let mut selected = false;
    let mut render = |ui: &mut egui::Ui| {
        let response = ui.allocate_response(vec2(180.0, 36.0), Sense::click());
        selected |= response.clicked();
        let context_response = if pinned {
            let pin_response = close_control(
                ui,
                egui::Rect::from_center_size(pos2(40.0, 20.0), vec2(16.0, 16.0)),
                ui.id().with("pin"),
                true,
                egui::Color32::GRAY,
                egui::Color32::WHITE,
                egui::Color32::GRAY,
            );
            response.union(pin_response)
        } else {
            response
        };
        if let Some(command) = tab_context_menu(
            &context_response,
            pinned,
            true,
            false,
            false,
            quick_access,
            matches!(
                expected,
                TabAction::ExpandFolderTree | TabAction::CollapseFolderTree
            )
            .then_some(expected),
            Some(true),
        ) {
            action = Some(command);
        }
    };
    let _ = ctx.run_ui(RawInput::default(), &mut render);
    let point = pos2(40.0, 20.0);
    let _ = ctx.run_ui(
        pointer_input(point, PointerButton::Secondary, true),
        &mut render,
    );
    let _ = ctx.run_ui(
        pointer_input(point, PointerButton::Secondary, false),
        &mut render,
    );
    // Popups use an initial sizing pass before exposing their painted rows.
    let output = ctx.run_ui(RawInput::default(), &mut render);
    let command_position = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::epaint::Shape::Text(text) if text.galley.text() == rust_i18n::t!(label) => {
                Some(shape.shape.visual_bounding_rect().center())
            }
            _ => None,
        })
        .expect("requested command is visible");
    if quick_access.is_none() {
        assert!(!output.shapes.iter().any(|shape| match &shape.shape {
            egui::epaint::Shape::Text(text) => {
                text.galley.text() == rust_i18n::t!("context_menu.pin_quick_access")
                    || text.galley.text() == rust_i18n::t!("context_menu.unpin_quick_access")
            }
            _ => false,
        }));
    }
    for pressed in [true, false] {
        let _ = ctx.run_ui(
            pointer_input(command_position, PointerButton::Primary, pressed),
            &mut render,
        );
    }
    assert_eq!(action, Some(expected));
    assert!(!selected);
}

#[test]
fn tab_control_is_clickable_without_selecting_or_dragging_the_tab() {
    for pinned in [false, true] {
        let ctx = Context::default();
        let mut clicked = false;
        let mut selected = false;
        let mut dragged = false;
        let mut render = |ui: &mut egui::Ui| {
            let tab_rect = egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(180.0, 36.0));
            let tab_response = ui.interact(tab_rect, ui.id().with("tab"), Sense::click_and_drag());
            selected |= tab_response.clicked();
            dragged |= tab_response.drag_started();
            let rect = egui::Rect::from_min_size(pos2(10.0, 10.0), vec2(16.0, 16.0));
            clicked |= close_control(
                ui,
                rect,
                ui.id().with("close"),
                pinned,
                egui::Color32::GRAY,
                egui::Color32::WHITE,
                egui::Color32::GRAY,
            )
            .clicked();
        };
        let _ = ctx.run_ui(RawInput::default(), &mut render);
        for pressed in [true, false] {
            let _ = ctx.run_ui(
                pointer_input(pos2(18.0, 18.0), PointerButton::Primary, pressed),
                &mut render,
            );
        }
        assert!(clicked);
        assert!(!selected);
        assert!(!dragged);
    }
}

#[test]
fn pinned_tab_control_uses_the_quick_access_icon() {
    let ctx = Context::default();
    let output = ctx.run_ui(RawInput::default(), |ui| {
        close_control(
            ui,
            egui::Rect::from_min_size(pos2(10.0, 10.0), vec2(16.0, 16.0)),
            ui.id().with("pin"),
            true,
            egui::Color32::GRAY,
            egui::Color32::WHITE,
            egui::Color32::GRAY,
        );
    });
    assert!(output.shapes.iter().any(|shape| matches!(
        &shape.shape,
        egui::epaint::Shape::Text(text) if text.galley.text() == "📌"
    )));
}
