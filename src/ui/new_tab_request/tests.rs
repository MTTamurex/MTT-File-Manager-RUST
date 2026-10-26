use super::*;

fn click_item(
    panel: ActivePanel,
    item: &FileEntry,
    enabled: bool,
    modifiers: egui::Modifiers,
) -> Option<NewTabRequest> {
    let ctx = egui::Context::default();
    let render = |ui: &mut egui::Ui| {
        set_render_panel(ui.ctx(), panel, enabled);
        let (_, response) =
            ui.allocate_exact_size(egui::vec2(100.0, 60.0), egui::Sense::click_and_drag());
        item_middle_click(&response, item, false);
        // Rendering the other pane must not change the captured destination.
        set_render_panel(ui.ctx(), panel.other(), true);
    };
    let _ = ctx.run_ui(egui::RawInput::default(), render);
    for pressed in [true, false] {
        let _ = ctx.run_ui(
            egui::RawInput {
                modifiers,
                events: vec![
                    egui::Event::PointerMoved(egui::pos2(20.0, 20.0)),
                    egui::Event::PointerButton {
                        pos: egui::pos2(20.0, 20.0),
                        button: egui::PointerButton::Middle,
                        pressed,
                        modifiers,
                    },
                ],
                ..Default::default()
            },
            render,
        );
    }
    let request = take(&ctx);
    assert!(
        take(&ctx).is_none(),
        "A request must be processed only once"
    );
    request
}

#[test]
fn middle_click_preserves_the_physical_origin_for_folders_and_archives() {
    for panel in [ActivePanel::Left, ActivePanel::Right] {
        for path in [r"C:\Folder", r"C:\archive.zip", r"C:\archive.zip\Folder"] {
            let item = FileEntry::from_path(path.into(), true);
            let request = click_item(panel, &item, true, egui::Modifiers::NONE)
                .expect("navigable item creates a request");
            assert_eq!(request.panel, panel);
            assert_eq!(request.paths, [path]);
        }
    }
}

#[test]
fn ordinary_files_and_blocked_panels_do_not_create_tabs() {
    let file = FileEntry::from_path(r"C:\file.txt".into(), false);
    assert!(click_item(ActivePanel::Left, &file, true, egui::Modifiers::NONE).is_none());
    let folder = FileEntry::from_path(r"C:\Folder".into(), true);
    assert!(click_item(ActivePanel::Right, &folder, false, egui::Modifiers::NONE).is_none());
}

#[test]
fn only_ctrl_overrides_focus_while_shift_keeps_the_configured_behavior() {
    let folder = FileEntry::from_path("C:\\".into(), true);
    for modifiers in [
        egui::Modifiers::NONE,
        egui::Modifiers::SHIFT,
        egui::Modifiers::CTRL,
        egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
    ] {
        let request = click_item(ActivePanel::Left, &folder, true, modifiers).unwrap();
        assert_eq!(request.ctrl, modifiers.ctrl);
        for activate in [false, true] {
            let prefs = crate::app::tab_preferences::TabPreferences {
                activate_new_tabs: activate,
                ..Default::default()
            };
            assert_eq!(
                prefs.should_activate_new_tab(request.ctrl),
                activate != modifiers.ctrl
            );
        }
    }
}

#[test]
fn deferred_open_preserves_ctrl_and_origin_after_rendering_the_other_panel() {
    let ctx = egui::Context::default();
    set_render_panel(&ctx, ActivePanel::Right, true);
    queue(&ctx, "C:\\", true);
    set_render_panel(&ctx, ActivePanel::Left, false);
    let request = take(&ctx).unwrap();
    assert_eq!(request.panel, ActivePanel::Right);
    assert!(request.ctrl);
    assert_eq!(request.paths, ["C:\\"]);
    assert!(take(&ctx).is_none());
    queue(&ctx, "D:\\", true);
    assert!(take(&ctx).is_none());
}

#[test]
fn batch_open_keeps_every_destination_and_the_origin_until_rendering_finishes() {
    for panel in [ActivePanel::Left, ActivePanel::Right] {
        for ctrl in [false, true] {
            let ctx = egui::Context::default();
            let paths = vec!["C:\\".to_owned(), "D:\\".to_owned(), "E:\\".to_owned()];
            set_render_panel(&ctx, panel, true);
            queue_paths(&ctx, paths.clone(), ctrl);
            set_render_panel(&ctx, panel.other(), true);
            let request = take(&ctx).unwrap();
            assert_eq!(request.paths, paths);
            assert_eq!(request.panel, panel);
            assert_eq!(request.ctrl, ctrl);
            assert!(take(&ctx).is_none());
        }
    }
}

#[test]
fn empty_selections_and_blocked_panels_do_not_queue_batch_opens() {
    let ctx = egui::Context::default();
    set_render_panel(&ctx, ActivePanel::Left, true);
    queue_paths(&ctx, Vec::new(), false);
    assert!(take(&ctx).is_none());
    set_render_panel(&ctx, ActivePanel::Right, false);
    queue_paths(&ctx, vec!["C:\\".to_owned(), "D:\\".to_owned()], false);
    assert!(take(&ctx).is_none());
}
