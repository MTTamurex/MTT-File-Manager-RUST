use super::*;
use crate::infrastructure::directory_cache::DirectoryCache;
use std::sync::Arc;

fn target(tree_id: egui::Id) -> SidebarFolderTarget {
    SidebarFolderTarget::new(tree_id, Path::new(r"C:\Cloud\Folder"), "Folder")
}

fn frame(ctx: &egui::Context, editing: &mut Option<SidebarFolderRename>, show_nodes: bool) {
    let tree = SidebarTreeState::new(Arc::new(DirectoryCache::new()));
    let mut icons = crate::ui::icon_loader::IconLoader::new();
    let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
        rename::begin_frame(ctx);
        if show_nodes {
            for tree_id in [egui::Id::new("drive_tree"), egui::Id::new("cloud_tree")] {
                let node = FolderNode {
                    path: target(tree_id).path,
                    name: "Folder".into(),
                    has_subfolders: Some(false),
                    is_hidden: false,
                };
                let mut context = SidebarTreeContext {
                    tree_state: &tree,
                    current_path: "D:\\",
                    icon_loader: &mut icons,
                    is_renaming: editing.is_some(),
                    is_item_dragging: false,
                    tree_id,
                    renaming: editing.as_ref(),
                };
                render_tree_node(ui, &node, 1, &mut context, &mut None);
            }
        }
        rename::finish_frame(ctx, editing);
    });
}

#[test]
fn sidebar_tree_aliases_only_create_an_editor_in_the_originating_tree() {
    let ctx = egui::Context::default();
    let drive = target(egui::Id::new("drive_tree"));
    let cloud = target(egui::Id::new("cloud_tree"));
    assert_eq!(drive.path, cloud.path);
    assert_ne!(drive.editor_id(), cloud.editor_id());
    let mut editing = Some(SidebarFolderRename::new(cloud.clone()));
    frame(&ctx, &mut editing, true);
    assert!(editing.is_some());
    assert!(egui::TextEdit::load_state(&ctx, cloud.editor_id()).is_some());
    assert!(egui::TextEdit::load_state(&ctx, drive.editor_id()).is_none());
}

#[test]
fn sidebar_tree_disappearing_node_cancels_editing_and_releases_keyboard_focus() {
    let ctx = egui::Context::default();
    let target = target(egui::Id::new("cloud_tree"));
    let mut editing = Some(SidebarFolderRename::new(target.clone()));
    frame(&ctx, &mut editing, true);
    assert!(ctx.memory(|memory| memory.has_focus(target.editor_id())));
    // A collapsed section, removed node, or different tab no longer renders it.
    frame(&ctx, &mut editing, false);
    assert!(editing.is_none());
    assert!(!ctx.memory(|memory| memory.has_focus(target.editor_id())));
}

#[test]
fn sidebar_tree_tab_handoff_can_cancel_before_the_next_sidebar_pass() {
    let ctx = egui::Context::default();
    let target = target(egui::Id::new("cloud_tree"));
    let mut editing = Some(SidebarFolderRename::new(target.clone()));
    frame(&ctx, &mut editing, true);
    rename::cancel(&ctx, &mut editing);
    assert!(editing.is_none());
    assert!(!ctx.memory(|memory| memory.has_focus(target.editor_id())));
    frame(&ctx, &mut editing, true);
    assert!(
        editing.is_none(),
        "the destination tab must not revive the editor"
    );
}
