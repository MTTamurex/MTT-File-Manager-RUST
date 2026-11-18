use super::*;
use crate::app::state::sidebar_tree_state::SidebarTreeState;
use crate::domain::file_tag::{FileTag, TagColor};
use crate::domain::folder_group::FolderGroup;
use crate::domain::pinned_folder::PinnedFolder;
use crate::infrastructure::directory_cache::DirectoryCache;
use rustc_hash::FxHashMap;
use std::collections::HashMap;
use std::sync::Arc;

fn content_height(
    pinned_count: usize,
    tag_count: usize,
    quick_access_visible: bool,
    collapsed: bool,
) -> f32 {
    with_sidebar(pinned_count, tag_count, collapsed, |ctx, sidebar| {
        let mut height = 0.0;
        let _ = ctx.run_ui(RawInput::default(), |ui| {
            ui.set_max_size(vec2(240.0, 220.0));
            height = render_content(ui, sidebar, quick_access_visible)
                .content_size
                .y;
        });
        height
    })
}

fn with_sidebar<R>(
    pinned_count: usize,
    tag_count: usize,
    collapsed: bool,
    run: impl FnOnce(&Context, &mut SidebarContext<'_>) -> R,
) -> R {
    with_sidebar_tree(pinned_count, tag_count, collapsed, false, run)
}

fn with_sidebar_tree<R>(
    pinned_count: usize,
    tag_count: usize,
    collapsed: bool,
    wide_tree: bool,
    run: impl FnOnce(&Context, &mut SidebarContext<'_>) -> R,
) -> R {
    with_sidebar_tree_and_groups(pinned_count, &[], tag_count, collapsed, wide_tree, run)
}

fn with_sidebar_and_groups<R>(
    pinned_count: usize,
    folder_groups: &[FolderGroup],
    tag_count: usize,
    collapsed: bool,
    run: impl FnOnce(&Context, &mut SidebarContext<'_>) -> R,
) -> R {
    with_sidebar_tree_and_groups(
        pinned_count,
        folder_groups,
        tag_count,
        collapsed,
        false,
        run,
    )
}

fn with_sidebar_tree_and_groups<R>(
    pinned_count: usize,
    folder_groups: &[FolderGroup],
    tag_count: usize,
    collapsed: bool,
    wide_tree: bool,
    run: impl FnOnce(&Context, &mut SidebarContext<'_>) -> R,
) -> R {
    let ctx = Context::default();
    let mut fonts = egui::FontDefinitions::default();
    fonts.families.insert(
        egui::FontFamily::Name("icons".into()),
        fonts.families[&egui::FontFamily::Proportional].clone(),
    );
    ctx.set_fonts(fonts);
    let pinned: Vec<_> = (0..pinned_count)
        .map(|index| PinnedFolder {
            path: format!(r"C:\Folders\Folder{index}"),
            display_name: format!("Folder {index}"),
            position: index as i64,
        })
        .collect();
    let tags: FxHashMap<_, _> = (0..tag_count)
        .map(|index| {
            (
                index as i64,
                FileTag {
                    id: index as i64,
                    name: format!("Tag {index}"),
                    color: TagColor::Blue,
                    position: index as i64,
                    parent_id: None,
                },
            )
        })
        .collect();
    let tag_ids: Vec<_> = (0..tag_count as i64).collect();
    let cache = Arc::new(DirectoryCache::new());
    let mut tree = SidebarTreeState::new(Arc::clone(&cache));
    let mut disks = Vec::new();
    if wide_tree {
        let name = "Folder ".repeat(30);
        cache.put(
            "C:\\".into(),
            vec![crate::domain::file_entry::FileEntry {
                path: format!(r"C:\{name}").into(),
                name,
                is_dir: true,
                size: 0,
                modified: 0,
                created: None,
                folder_cover: None,
                drive_info: None,
                sync_status: crate::domain::file_entry::SyncStatus::default(),
                is_hidden: false,
                recycle_bin: None,
            }],
        );
        tree.toggle_expand(std::path::Path::new("C:\\"));
        disks.push(("C:\\".into(), "Local Disk".into()));
    }
    let drive_info = HashMap::new();
    let mounted = HashMap::new();
    let tag_counts = FxHashMap::default();
    let subtree_tag_counts = FxHashMap::default();
    let collapsed_tags = rustc_hash::FxHashSet::default();
    let mut icons = crate::ui::icon_loader::IconLoader::new();
    let mut sidebar = SidebarContext {
        disks: &disks,
        drive_info_cache: &drive_info,
        cloud_roots: &[],
        network_locations: &[],
        current_path: "C:\\",
        context_selection: None,
        is_computer_view: true,
        is_recycle_bin_view: false,
        computer_icon: None,
        is_renaming: false,
        icon_loader: &mut icons,
        pinned_folders: &pinned,
        folder_groups,
        is_item_dragging: false,
        can_drop_items_to_recycle_bin: false,
        is_folder_dragging: false,
        dragging_path: None,
        show_recycle_bin: false,
        show_quick_access: true,
        show_tags: true,
        collapse_quick_access: collapsed,
        collapse_cloud_drives: collapsed,
        collapse_local_disks: collapsed,
        collapse_network_drives: collapsed,
        collapse_network_locations: collapsed,
        sidebar_renaming: None,
        sidebar_rename_focus: false,
        folder_rename: None,
        mounted_iso_drives: &mounted,
        tree_state: &tree,
        tag_definitions: &tags,
        sidebar_tag_ids: &tag_ids,
        tag_counts: &tag_counts,
        subtree_tag_counts: &subtree_tag_counts,
        sidebar_tags_collapsed: &collapsed_tags,
        active_tag_filter: None,
        collapse_tags: collapsed,
    };
    run(&ctx, &mut sidebar)
}

#[test]
fn adding_quick_access_items_and_tags_grows_the_same_scrollable_content() {
    let initial = content_height(2, 2, true, false);
    let more_pinned = content_height(22, 2, true, false);
    let more_tags = content_height(2, 22, true, false);
    assert!(more_pinned > initial + 500.0);
    assert!(more_tags > initial + 500.0);
}

#[test]
fn horizontal_quick_access_does_not_reserve_space_in_the_sidebar() {
    assert_eq!(
        content_height(2, 2, false, false),
        content_height(22, 2, false, false)
    );
    assert!(content_height(22, 2, true, false) > content_height(22, 2, false, false) + 500.0);
}

#[test]
fn collapsed_sections_fit_their_headers_regardless_of_item_count() {
    assert_eq!(
        content_height(2, 2, true, true),
        content_height(22, 22, true, true)
    );
}

#[test]
fn dropping_a_folder_on_tags_does_not_pin_it_to_quick_access() {
    assert!(!matches!(
        drop_folder_on("Tag 0"),
        Some(SidebarAction::PinFolder(_))
    ));
}

#[test]
fn dropping_a_folder_on_quick_access_still_pins_it() {
    assert!(matches!(
        drop_folder_on(&rust_i18n::t!("sidebar.quick_access")),
        Some(SidebarAction::PinFolder(path)) if path == r"C:\Folders\New Folder"
    ));
}

#[test]
fn saved_group_can_be_reordered_before_a_pinned_folder_in_vertical_quick_access() {
    let group = FolderGroup {
        id: 7,
        name: "Saved group".to_owned(),
        position: 1,
        paths: Vec::new(),
    };
    with_sidebar_and_groups(1, &[group], 0, false, |ctx, sidebar| {
        let screen_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(240.0, 500.0));
        let mut render = |events| {
            let mut action = None;
            let output = ctx.run_ui(
                RawInput {
                    screen_rect: Some(screen_rect),
                    events,
                    ..RawInput::default()
                },
                |ui| {
                    ui.set_max_size(vec2(240.0, 500.0));
                    action = render_content(ui, sidebar, true).inner;
                },
            );
            (output, action)
        };
        let text_position = |output: &egui::FullOutput, text: &str| {
            output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::epaint::Shape::Text(text_shape) if text_shape.galley.text() == text => {
                        Some(shape.shape.visual_bounding_rect().center())
                    }
                    _ => None,
                })
                .expect("Quick Access item text is visible")
        };

        let (initial, _) = render(Vec::new());
        let group_position = text_position(&initial, "Saved group");
        let folder_position = text_position(&initial, "Folder 0");
        let drop_position = egui::Pos2::new(group_position.x, folder_position.y - 8.0);
        let press = egui::Event::PointerButton {
            pos: group_position,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        };
        render(vec![egui::Event::PointerMoved(group_position), press]);
        render(vec![egui::Event::PointerMoved(drop_position)]);
        let (_, action) = render(vec![
            egui::Event::PointerMoved(drop_position),
            egui::Event::PointerButton {
                pos: drop_position,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
        ]);

        assert!(matches!(
            action,
            Some(SidebarAction::ReorderQuickAccessItem { from: 1, to: 0 })
        ));
    });
}

fn drop_folder_on(label: &str) -> Option<SidebarAction> {
    with_sidebar(0, 1, false, |ctx, sidebar| {
        sidebar.is_item_dragging = true;
        sidebar.is_folder_dragging = true;
        sidebar.dragging_path = Some(r"C:\Folders\New Folder");
        release_on_label(ctx, sidebar, label, false)
    })
}

#[test]
fn releasing_a_pinned_folder_over_tags_does_not_reorder_quick_access() {
    let action = with_sidebar(2, 1, false, |ctx, sidebar| {
        release_on_label(ctx, sidebar, "Tag 0", true)
    });
    assert!(!matches!(
        action,
        Some(SidebarAction::ReorderQuickAccessItem { .. })
    ));
}

#[test]
fn wide_folder_trees_do_not_push_section_toggles_outside_the_viewport() {
    with_sidebar_tree(0, 1, false, true, |ctx, sidebar| {
        let output = ctx.run_ui(RawInput::default(), |ui| {
            ui.set_max_size(vec2(240.0, 500.0));
            let area = render_content(ui, sidebar, true);
            assert!(area.content_size.x > area.inner_rect.width());
        });
        let toggles: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Path(path)
                    if path.closed
                        && path.points.len() == 3
                        && shape.shape.visual_bounding_rect().width()
                            > crate::ui::disclosure::SIDEBAR_SIZE
                        && shape.shape.visual_bounding_rect().height()
                            > crate::ui::disclosure::SIDEBAR_SIZE =>
                {
                    Some(shape.shape.visual_bounding_rect())
                }
                _ => None,
            })
            .collect();
        assert!(toggles.len() >= 2);
        for toggle in toggles {
            assert!(
                toggle.right() <= 240.0,
                "toggle outside viewport: {toggle:?}"
            );
        }
    });
}

fn release_on_label(
    ctx: &Context,
    sidebar: &mut SidebarContext<'_>,
    label: &str,
    reorder: bool,
) -> Option<SidebarAction> {
    let output = ctx.run_ui(RawInput::default(), |ui| {
        ui.set_max_size(vec2(240.0, 500.0));
        render_content(ui, sidebar, true);
    });
    let pos = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::epaint::Shape::Text(text) if text.galley.text() == label => {
                Some(shape.shape.visual_bounding_rect().center())
            }
            _ => None,
        })
        .expect("drop target is visible");
    let mut action = None;
    for pressed in [true, false] {
        let _ = ctx.run_ui(
            RawInput {
                events: vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                ..Default::default()
            },
            |ui| {
                ui.set_max_size(vec2(240.0, 500.0));
                action = render_content(ui, sidebar, true).inner;
            },
        );
        if pressed && reorder {
            ctx.data_mut(|data| {
                data.insert_temp(egui::Id::new("qa_reorder_drag"), (0usize, 14.0f32))
            });
        }
    }
    action
}
