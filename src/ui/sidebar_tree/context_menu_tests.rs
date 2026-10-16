use super::*;
use crate::infrastructure::directory_cache::DirectoryCache;
use std::sync::Arc;

fn right_click(depth: usize, renaming: bool, dragging: bool) -> Option<SidebarTreeAction> {
    let ctx = egui::Context::default();
    let tree = SidebarTreeState::new(Arc::new(DirectoryCache::new()));
    let node = FolderNode {
        path: r"C:\Folder\Subfolder".into(),
        name: "Subfolder".into(),
        has_subfolders: Some(true),
        is_hidden: false,
    };
    let mut icons = crate::ui::icon_loader::IconLoader::new();
    let mut render = |ui: &mut egui::Ui| {
        let mut action = None;
        let mut context = SidebarTreeContext {
            tree_state: &tree,
            current_path: r"C:\Other",
            icon_loader: &mut icons,
            is_renaming: renaming,
            is_item_dragging: dragging,
            renaming: None,
            tree_id: egui::Id::new("drive_tree").with("C:\\"),
        };
        render_tree_node(ui, &node, depth, &mut context, &mut action);
        action
    };
    let output = ctx.run_ui(egui::RawInput::default(), |ui| {
        let _ = render(ui);
    });
    let pos = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::epaint::Shape::Text(text) if text.galley.text() == "Subfolder" => {
                Some(shape.shape.visual_bounding_rect().center())
            }
            _ => None,
        })
        .expect("node is visible");
    let mut result = None;
    for pressed in [true, false] {
        let _ = ctx.run_ui(
            egui::RawInput {
                events: vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Secondary,
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
    assert!(!tree.is_expanded(&node.path));
    result
}

#[test]
fn sidebar_tree_right_click_targets_the_node_without_navigating_or_expanding() {
    for depth in [1, 3] {
        assert!(matches!(right_click(depth, false, false),
            Some(SidebarTreeAction::OpenContextMenu(target)) if target.path == Path::new(r"C:\Folder\Subfolder")));
    }
}

#[test]
fn sidebar_tree_context_menu_is_suppressed_during_drag_or_rename() {
    assert!(right_click(1, true, false).is_none());
    assert!(right_click(1, false, true).is_none());
}
