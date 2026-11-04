use super::*;

#[test]
fn direct_hover_switches_away_from_a_sibling_with_active_descendants() {
    let ctx = egui::Context::default();
    let mut icons = SvgIconManager::new();
    let item = ContextMenuItem::new(-299, "Group by")
        .with_subitems(vec![ContextMenuItem::new(-300, "Name")]);
    let mut hover_position = egui::Pos2::ZERO;

    // Establish the widget geometry before delivering a pointer event.
    let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
        ui.set_width(220.0);
        hover_position = ui.cursor().min + egui::vec2(50.0, 15.0);
        render_single_item(ui, &item, &mut None, 0, &mut None, &mut icons);
    });

    SUBMENU_HIERARCHY.with(|hierarchy| {
        *hierarchy.borrow_mut() = vec![Some(-99), Some(42)];
    });
    let input = egui::RawInput {
        events: vec![egui::Event::PointerMoved(hover_position)],
        ..Default::default()
    };
    let _ = ctx.run_ui(input, |ui| {
        ui.set_width(220.0);
        render_single_item(ui, &item, &mut None, 0, &mut None, &mut icons);
    });

    let hierarchy = SUBMENU_HIERARCHY.with(|hierarchy| hierarchy.take());
    assert_eq!(hierarchy, vec![Some(-299)]);
}

#[test]
fn populated_fallback_requests_pending_shell_items_and_retries_on_hover() {
    let ctx = egui::Context::default();
    let mut icons = SvgIconManager::new();
    let item = ContextMenuItem {
        has_pending_submenu: true,
        ..ContextMenuItem::new(42, "New").with_subitems(vec![ContextMenuItem::new(-1, "Folder")])
    };
    let mut hover_position = egui::Pos2::ZERO;
    let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
        ui.set_width(220.0);
        hover_position = ui.cursor().min + egui::vec2(50.0, 15.0);
        render_single_item(ui, &item, &mut None, 0, &mut None, &mut icons);
    });

    // If dispatch fails, the next frame must request again despite the local child.
    for _ in 0..2 {
        let mut lazy_load = None;
        let input = egui::RawInput {
            events: vec![egui::Event::PointerMoved(hover_position)],
            ..Default::default()
        };
        let _ = ctx.run_ui(input, |ui| {
            ui.set_width(220.0);
            render_single_item(ui, &item, &mut None, 0, &mut lazy_load, &mut icons);
        });
        assert_eq!(lazy_load, Some(42));
    }
    SUBMENU_HIERARCHY.with(|hierarchy| hierarchy.borrow_mut().clear());
}

#[test]
fn populated_submenu_does_not_request_shell_items_after_loading() {
    let ctx = egui::Context::default();
    let mut icons = SvgIconManager::new();
    let item =
        ContextMenuItem::new(42, "New").with_subitems(vec![ContextMenuItem::new(-1, "Folder")]);
    let mut hover_position = egui::Pos2::ZERO;
    let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
        ui.set_width(220.0);
        hover_position = ui.cursor().min + egui::vec2(50.0, 15.0);
        render_single_item(ui, &item, &mut None, 0, &mut None, &mut icons);
    });
    let mut lazy_load = None;
    let input = egui::RawInput {
        events: vec![egui::Event::PointerMoved(hover_position)],
        ..Default::default()
    };
    let _ = ctx.run_ui(input, |ui| {
        ui.set_width(220.0);
        render_single_item(ui, &item, &mut None, 0, &mut lazy_load, &mut icons);
    });
    assert_eq!(lazy_load, None);
    SUBMENU_HIERARCHY.with(|hierarchy| assert_eq!(hierarchy.borrow()[0], Some(42)));
    SUBMENU_HIERARCHY.with(|hierarchy| hierarchy.borrow_mut().clear());
}
