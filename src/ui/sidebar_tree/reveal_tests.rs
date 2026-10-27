use super::*;
use crate::infrastructure::directory_cache::DirectoryCache;
use std::sync::Arc;

#[test]
fn reveal_scrolls_once_without_changing_keyboard_focus_in_both_layouts() {
    for independent in [false, true] {
        let ctx = egui::Context::default();
        let mut tree = SidebarTreeState::new(Arc::new(DirectoryCache::new()));
        let root = Path::new("C:\\");
        let focus = egui::Id::new("content_focus");
        let mut offset = 0.0;
        let mut frame = |tree: &SidebarTreeState, ctx: &egui::Context| {
            let mut output = None;
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.set_max_size(egui::vec2(240.0, 180.0));
                ui.memory_mut(|memory| memory.request_focus(focus));
                let area = egui::ScrollArea::both().max_height(160.0);
                let area = if independent {
                    area.vertical_scroll_offset(offset)
                } else {
                    area
                };
                output = Some(area.show(ui, |ui| {
                    ui.allocate_space(egui::vec2(200.0, 600.0));
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(200.0, 24.0), Sense::hover());
                    reveal_row(ui, tree, root, root, rect);
                }));
                assert_eq!(ui.memory(|memory| memory.focused()), Some(focus));
            });
            let output = output.unwrap();
            offset = output.state.offset.y;
            offset
        };
        frame(&tree, &ctx);
        tree.request_reveal(root, root);
        frame(&tree, &ctx);
        assert!(frame(&tree, &ctx) > 0.0);
        assert!(!tree.take_reveal_row(root, root));
    }
}

#[test]
fn reveal_makes_a_deep_folder_name_visible_horizontally() {
    let ctx = egui::Context::default();
    let mut tree = SidebarTreeState::new(Arc::new(DirectoryCache::new()));
    let root = Path::new("C:\\Folder");
    let node = FolderNode {
        path: root.into(),
        name: "Folder".into(),
        has_subfolders: Some(false),
        is_hidden: false,
    };
    let mut icons = crate::ui::icon_loader::IconLoader::new();
    let mut horizontal_offset = 0.0;
    let mut render = |tree: &SidebarTreeState| {
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.set_max_size(egui::vec2(200.0, 160.0));
            let output = egui::ScrollArea::both().show(ui, |ui| {
                let mut context = SidebarTreeContext {
                    tree_state: tree,
                    current_path: "C:\\Folder",
                    icon_loader: &mut icons,
                    is_renaming: false,
                    is_item_dragging: false,
                    tree_id: egui::Id::new("tree"),
                    renaming: None,
                    context_selection: None,
                };
                render_tree_node(ui, root, &node, 20, &mut context, &mut None);
            });
            horizontal_offset = output.state.offset.x;
        });
        horizontal_offset
    };
    render(&tree);
    tree.request_reveal(root, root);
    render(&tree);
    assert!(
        render(&tree) > 0.0,
        "the name must not remain clipped behind indentation"
    );
}
