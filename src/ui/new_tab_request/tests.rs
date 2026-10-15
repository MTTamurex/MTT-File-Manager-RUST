use super::*;

fn click_item(panel: ActivePanel, item: &FileEntry, enabled: bool) -> Option<NewTabRequest> {
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
                events: vec![
                    egui::Event::PointerMoved(egui::pos2(20.0, 20.0)),
                    egui::Event::PointerButton {
                        pos: egui::pos2(20.0, 20.0),
                        button: egui::PointerButton::Middle,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
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
            let request = click_item(panel, &item, true).expect("navigable item creates a request");
            assert_eq!(request.panel, panel);
            assert_eq!(request.path, path);
        }
    }
}

#[test]
fn ordinary_files_and_blocked_panels_do_not_create_tabs() {
    let file = FileEntry::from_path(r"C:\file.txt".into(), false);
    assert!(click_item(ActivePanel::Left, &file, true).is_none());
    let folder = FileEntry::from_path(r"C:\Folder".into(), true);
    assert!(click_item(ActivePanel::Right, &folder, false).is_none());
}
