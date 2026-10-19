use super::*;
use crate::app::state::sidebar_tree_state::SidebarTreeState;
use crate::application::sidebar_tree::{SidebarFolderRename, SidebarFolderTarget};
use crate::domain::cloud_root::CloudRoot;
use crate::infrastructure::directory_cache::DirectoryCache;
use std::sync::Arc;

fn frame(
    ctx: &egui::Context,
    editing: &mut Option<SidebarFolderRename>,
    events: Vec<egui::Event>,
) -> (egui::FullOutput, Option<SidebarAction>) {
    let roots = vec![CloudRoot::windows_cloud_files(
        r"C:\Cloud".into(),
        "Cloud Provider".into(),
        None,
    )];
    let tree = SidebarTreeState::new(Arc::new(DirectoryCache::new()));
    assert!(tree.get_children(Path::new("C:\\")).is_none());
    let mut icons = crate::ui::icon_loader::IconLoader::new();
    let drive_info = Default::default();
    let mounted = Default::default();
    let tags = Default::default();
    let counts = Default::default();
    let mut action = None;
    let output = ctx.run_ui(
        egui::RawInput {
            events,
            ..Default::default()
        },
        |ui| {
            crate::ui::sidebar_tree::rename::begin_frame(ctx);
            ui.set_width(300.0);
            let mut sidebar = SidebarContext {
                disks: &[],
                drive_info_cache: &drive_info,
                cloud_roots: &roots,
                network_locations: &[],
                current_path: "D:\\",
                context_selection: None,
                is_computer_view: false,
                is_recycle_bin_view: false,
                computer_icon: None,
                is_renaming: editing.is_some(),
                icon_loader: &mut icons,
                pinned_folders: &[],
                is_item_dragging: false,
                can_drop_items_to_recycle_bin: false,
                is_folder_dragging: false,
                dragging_path: None,
                show_recycle_bin: false,
                show_quick_access: false,
                show_tags: false,
                collapse_quick_access: false,
                collapse_cloud_drives: false,
                collapse_local_disks: false,
                collapse_network_drives: false,
                collapse_network_locations: false,
                sidebar_renaming: None,
                sidebar_rename_focus: false,
                folder_rename: editing.as_ref(),
                mounted_iso_drives: &mounted,
                tree_state: &tree,
                tag_definitions: &tags,
                sidebar_tag_ids: &[],
                tag_counts: &counts,
                active_tag_filter: None,
                collapse_tags: false,
            };
            render_cloud_roots(ui, &mut sidebar, &mut action, 300.0);
            crate::ui::sidebar_tree::rename::finish_frame(ctx, editing);
        },
    );
    (output, action)
}

#[test]
fn sidebar_cloud_root_menu_and_editor_work_without_a_cached_parent_listing() {
    let ctx = egui::Context::default();
    let mut editing = None;
    let (output, _) = frame(&ctx, &mut editing, vec![]);
    let position = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::epaint::Shape::Text(text) if text.galley.text() == "Cloud Provider" => {
                Some(shape.shape.visual_bounding_rect().center())
            }
            _ => None,
        })
        .unwrap();
    let mut action = None;
    for pressed in [true, false] {
        (_, action) = frame(
            &ctx,
            &mut editing,
            vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Secondary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    let Some(SidebarAction::OpenTreeContextMenu(target)) = action else {
        panic!("cloud root must carry an explicit folder target")
    };
    assert_eq!(target.name, "Cloud");
    assert_eq!(target.path, Path::new(r"C:\Cloud"));
    let expected = SidebarFolderTarget::new(
        egui::Id::new("cloud_tree").with(r"C:\Cloud".to_string()),
        Path::new(r"C:\Cloud"),
        "Cloud",
    );
    assert_eq!(target.node_id, expected.node_id);
    editing = Some(SidebarFolderRename::new(target.clone()));
    frame(&ctx, &mut editing, vec![]);
    assert!(editing.is_some(), "root editor must actually render");
    assert!(egui::TextEdit::load_state(&ctx, target.editor_id()).is_some());
    let (_, action) = frame(
        &ctx,
        &mut editing,
        vec![egui::Event::Text("Renamed".into())],
    );
    assert!(matches!(action, Some(SidebarAction::TreeRenameChanged(text)) if text == "Renamed"));
}
