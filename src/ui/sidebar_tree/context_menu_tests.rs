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
            context_selection: None,
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
        let output = ctx.run_ui(
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
        if !pressed && !renaming && !dragging {
            let color = crate::ui::theme::selection_color(ctx.global_style().visuals.dark_mode);
            assert!(
                output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::epaint::Shape::Rect(rect) if rect.fill == color && rect.rect.contains(pos))),
                "right-clicked node must be highlighted in the click frame"
            );
        }
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

#[test]
fn sidebar_context_highlight_follows_the_exact_node_and_restores_navigation_after_closing() {
    let ctx = egui::Context::default();
    let tree = SidebarTreeState::new(Arc::new(DirectoryCache::new()));
    let mut icons = crate::ui::icon_loader::IconLoader::new();
    let drive = egui::Id::new("drive_tree");
    let cloud = egui::Id::new("cloud_tree");
    let rows = [
        (r"C:\Current", "Current", drive),
        (r"C:\Folder", "Folder", drive),
        (r"C:\Folder", "Folder", cloud),
    ];
    let mut render = |menu: &crate::application::context_menu::ContextMenuState| {
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            for (path, name, tree_id) in rows {
                let node = FolderNode {
                    path: path.into(),
                    name: name.into(),
                    has_subfolders: Some(false),
                    is_hidden: false,
                };
                let mut context = SidebarTreeContext {
                    tree_state: &tree,
                    current_path: r"C:\Current",
                    icon_loader: &mut icons,
                    is_renaming: false,
                    is_item_dragging: false,
                    tree_id,
                    renaming: None,
                    context_selection:
                        crate::ui::sidebar_selection::SidebarContextSelection::from_menu(menu),
                };
                render_tree_node(ui, &node, 1, &mut context, &mut None);
            }
        });
        let color = crate::ui::theme::selection_color(ctx.global_style().visuals.dark_mode);
        output.shapes.iter().filter_map(|shape| match &shape.shape {
            egui::epaint::Shape::Text(text) if matches!(text.galley.text(), "Current" | "Folder") => {
                let center = shape.shape.visual_bounding_rect().center();
                Some(output.shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::epaint::Shape::Rect(rect) if rect.fill == color && rect.rect.contains(center))))
            }
            _ => None,
        }).collect::<Vec<_>>()
    };
    let mut menu = crate::application::context_menu::ContextMenuState::default();
    assert_eq!(render(&menu), [true, false, false]);
    menu.open_for_sidebar_tree(egui::Pos2::ZERO, r"C:\Folder".into());
    menu.sidebar_folder_target = Some(SidebarFolderTarget::new(
        cloud,
        Path::new(r"C:\Folder"),
        "Folder",
    ));
    assert_eq!(render(&menu), [false, false, true]);
    menu.close();
    assert_eq!(render(&menu), [true, false, false]);
}
