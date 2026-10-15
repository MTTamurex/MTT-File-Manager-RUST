use super::*;
use crate::infrastructure::directory_cache::DirectoryCache;
use std::sync::Arc;

#[test]
fn middle_click_on_a_folder_node_opens_it_without_toggling_the_tree() {
    let ctx = egui::Context::default();
    let tree = SidebarTreeState::new(Arc::new(DirectoryCache::new()));
    let node = FolderNode {
        path: r"C:\Folder".into(),
        name: "Folder".into(),
        has_subfolders: Some(true),
        is_hidden: false,
    };
    let mut icons = crate::ui::icon_loader::IconLoader::new();
    let mut render = |ui: &mut egui::Ui| {
        let mut action = None;
        let mut tree_ctx = SidebarTreeContext {
            tree_state: &tree,
            current_path: "C:\\",
            icon_loader: &mut icons,
            is_renaming: false,
            is_item_dragging: false,
        };
        render_tree_node(ui, &node, 1, &mut tree_ctx, &mut action);
        action
    };
    let output = ctx.run_ui(egui::RawInput::default(), |ui| {
        let _ = render(ui);
    });
    let pos = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::epaint::Shape::Text(text) if text.galley.text() == "Folder" => {
                Some(shape.shape.visual_bounding_rect().center())
            }
            _ => None,
        })
        .expect("folder node is visible");
    let mut result = None;
    for pressed in [true, false] {
        let _ = ctx.run_ui(
            egui::RawInput {
                events: vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Middle,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                ..Default::default()
            },
            |ui| {
                result = render(ui);
            },
        );
    }
    assert!(matches!(result, Some(SidebarTreeAction::OpenInNewTab(path)) if path == r"C:\Folder"));
    assert!(!tree.is_expanded(&node.path));
}
