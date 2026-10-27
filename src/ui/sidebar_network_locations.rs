use crate::application::sidebar_tree::SidebarFolderTarget;
use crate::ui::sidebar::{allocate_sidebar_viewport_row, SidebarAction, SidebarContext};
use eframe::egui::{self, Color32, Pos2, Rect, Sense};
use rust_i18n::t;
use std::path::Path;

pub(super) fn render_network_locations(
    ui: &mut egui::Ui,
    ctx: &mut SidebarContext,
    action: &mut Option<SidebarAction>,
    width: f32,
) {
    if ctx.network_locations.is_empty() {
        return;
    }
    let (header, _) = allocate_sidebar_viewport_row(ui, width, 16.0, Sense::hover());
    if let Some(command) = crate::ui::sidebar::section_menu::header_menu(
        ui,
        header,
        "network_locations",
        ctx.is_renaming || ctx.is_item_dragging,
    ) {
        *action = Some(command);
    }
    let toggle_rect = Rect::from_center_size(
        Pos2::new(header.max.x - 12.0, header.center().y),
        egui::vec2(18.0, 18.0),
    );
    let toggle_response = ui
        .interact(
            toggle_rect,
            egui::Id::new("sidebar_toggle_network_locations"),
            Sense::click(),
        )
        .on_hover_text(if ctx.collapse_network_locations {
            t!("sidebar.expand_section")
        } else {
            t!("sidebar.collapse_section")
        });
    if ui.is_rect_visible(header) {
        ui.painter().text(
            Pos2::new(header.min.x + 8.0, header.center().y),
            egui::Align2::LEFT_CENTER,
            t!("sidebar.network_locations"),
            egui::FontId::proportional(10.0),
            Color32::from_gray(120),
        );
        ui.painter().text(
            toggle_rect.center(),
            egui::Align2::CENTER_CENTER,
            if ctx.collapse_network_locations {
                "v"
            } else {
                "^"
            },
            egui::FontId::proportional(14.0),
            if toggle_response.hovered() {
                ui.visuals().text_color()
            } else {
                Color32::from_gray(140)
            },
        );
    }
    if toggle_response.clicked() && action.is_none() {
        *action = Some(SidebarAction::ToggleNetworkLocations);
    }
    if ctx.collapse_network_locations {
        ui.add_space(6.0);
        return;
    }
    ui.add_space(4.0);
    for location in ctx.network_locations {
        let tree_id = ui
            .id()
            .with("network_locations")
            .with(&location.shortcut_path);
        let target =
            SidebarFolderTarget::new(tree_id, Path::new(&location.shortcut_path), &location.label);
        let editing = ctx.folder_rename.filter(|editing| editing.matches(&target));
        let expanded = ctx.tree_state.is_expanded(Path::new(&location.path));
        let (rect, response) = allocate_sidebar_viewport_row(ui, width, 28.0, Sense::click());
        let root_path = Path::new(&location.path);
        crate::ui::sidebar_tree::reveal_row(ui, ctx.tree_state, root_path, root_path, rect);
        let navigation_selected = !ctx.is_computer_view
            && !ctx.is_recycle_bin_view
            && path_is_at_or_under(ctx.current_path, &location.path);
        let selected = crate::ui::sidebar_selection::row_is_selected(
            ctx.context_selection,
            &target.path,
            Some(target.node_id),
            navigation_selected,
        );
        let arrow_zone = Rect::from_min_size(rect.min, egui::vec2(18.0, rect.height()));
        let label_rect = Rect::from_min_max(Pos2::new(rect.min.x + 48.0, rect.min.y), rect.max);
        if ui.is_rect_visible(rect) {
            if selected {
                ui.painter()
                    .rect_filled(rect, 0.0, crate::ui::theme::selection_color(ui.ctx()));
            } else if response.hovered() && !ctx.is_item_dragging {
                ui.painter().rect_filled(
                    rect,
                    0.0,
                    crate::ui::theme::selection_hover_color(ui.ctx()),
                );
            }
            ui.painter().text(
                arrow_zone.center(),
                egui::Align2::CENTER_CENTER,
                if expanded { "v" } else { ">" },
                egui::FontId::proportional(10.0),
                Color32::from_gray(140),
            );
            let icon = ctx
                .icon_loader
                .get_or_load_network_location_icon(ui.ctx())
                .or_else(|| ctx.icon_loader.folder_icon().cloned());
            if let Some(icon) = icon {
                let icon_rect = Rect::from_center_size(
                    Pos2::new(rect.min.x + 28.0, rect.center().y),
                    egui::vec2(16.0, 16.0),
                );
                ui.painter().image(
                    icon.id(),
                    icon_rect,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            if editing.is_none() {
                ui.painter().with_clip_rect(label_rect).text(
                    label_rect.left_center(),
                    egui::Align2::LEFT_CENTER,
                    &location.label,
                    egui::FontId::proportional(11.5),
                    if selected {
                        crate::ui::theme::selection_text_color(ui.ctx())
                    } else {
                        ui.visuals().text_color()
                    },
                );
            }
        }
        if let Some(editing) = editing {
            if let Some(rename_action) =
                crate::ui::sidebar_tree::rename::render(ui, label_rect, editing)
            {
                *action = Some(rename_action.into());
            }
        }
        if action.is_none() && !ctx.is_renaming && !ctx.is_item_dragging {
            if response.secondary_clicked() {
                *action = Some(SidebarAction::OpenTreeContextMenu(target));
            } else if response.middle_clicked() {
                *action = Some(SidebarAction::OpenInNewTab(location.path.clone()));
            } else if response.clicked() {
                if ui.input(|input| {
                    input
                        .pointer
                        .interact_pos()
                        .is_some_and(|point| arrow_zone.contains(point))
                }) {
                    *action = Some(SidebarAction::TreeToggleExpand(
                        location.path.clone().into(),
                    ));
                } else {
                    *action = Some(SidebarAction::NavigateTo(location.path.clone()));
                }
            }
        }
        if ctx.is_item_dragging
            && action.is_none()
            && ui.input(|input| {
                input.pointer.primary_released()
                    && input
                        .pointer
                        .hover_pos()
                        .is_some_and(|point| rect.contains(point))
            })
        {
            *action = Some(SidebarAction::DropItemsTo(location.path.clone()));
        }
        ui.add_space(2.0);
        if expanded {
            let mut tree_ctx = crate::ui::sidebar_tree::SidebarTreeContext {
                tree_state: ctx.tree_state,
                current_path: ctx.current_path,
                icon_loader: ctx.icon_loader,
                is_renaming: ctx.is_renaming,
                is_item_dragging: ctx.is_item_dragging,
                renaming: ctx.folder_rename,
                context_selection: ctx.context_selection,
                tree_id,
            };
            if let Some(tree_action) =
                crate::ui::sidebar_tree::render_drive_tree(ui, &location.path, &mut tree_ctx)
            {
                if action.is_none() {
                    *action = Some(tree_action.into());
                }
            }
        }
    }
    ui.add_space(6.0);
}

fn path_is_at_or_under(path: &str, root: &str) -> bool {
    let path = crate::domain::network_location::network_location_path_key(Path::new(path));
    let root = crate::domain::network_location::network_location_path_key(Path::new(root));
    path == root || path.starts_with(&format!(r"{root}\"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_toggle_hides_rows_and_preserves_tree_expansion() {
        use crate::app::state::sidebar_tree_state::SidebarTreeState;
        use crate::infrastructure::directory_cache::DirectoryCache;
        use std::sync::Arc;

        let ui_ctx = egui::Context::default();
        let target = tempfile::tempdir().unwrap();
        let locations = [crate::domain::network_location::NetworkLocation {
            kind: crate::domain::network_location::NetworkLocationKind::FolderShortcut,
            shortcut_path: r"C:\Network Shortcuts\Folder".into(),
            path: target.path().to_string_lossy().into_owned(),
            label: "Network Folder".into(),
        }];
        let mut tree = SidebarTreeState::new(Arc::new(DirectoryCache::new()));
        // Use a local temporary target, never probe a remote share in this test.
        tree.toggle_expand(target.path());
        let drive_info = Default::default();
        let mounted = Default::default();
        let tags = Default::default();
        let counts = Default::default();
        let mut icons = crate::ui::icon_loader::IconLoader::new();
        let mut sidebar = SidebarContext {
            disks: &[],
            drive_info_cache: &drive_info,
            cloud_roots: &[],
            network_locations: &locations,
            current_path: "C:\\",
            context_selection: None,
            is_computer_view: true,
            is_recycle_bin_view: false,
            computer_icon: None,
            is_renaming: false,
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
            folder_rename: None,
            mounted_iso_drives: &mounted,
            tree_state: &tree,
            tag_definitions: &tags,
            sidebar_tag_ids: &[],
            tag_counts: &counts,
            active_tag_filter: None,
            collapse_tags: false,
        };
        let mut render = |input, collapsed| {
            sidebar.collapse_network_locations = collapsed;
            let mut action = None;
            let mut top = 0.0;
            let mut bottom = 0.0;
            let output = ui_ctx.run_ui(input, |ui| {
                top = ui.cursor().top();
                render_network_locations(ui, &mut sidebar, &mut action, 260.0);
                bottom = ui.cursor().top();
            });
            (output, action, bottom - top)
        };
        let (expanded, _, expanded_height) = render(egui::RawInput::default(), false);
        assert!(expanded.shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::epaint::Shape::Text(text) if text.galley.text() == "Network Folder")
        }));
        let (collapsed, _, collapsed_height) = render(egui::RawInput::default(), true);
        assert!(collapsed_height < expanded_height);
        assert!(!collapsed.shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::epaint::Shape::Text(text) if text.galley.text() == "Network Folder")
        }));
        let toggle = collapsed
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) if text.galley.text() == "v" => {
                    Some(shape.shape.visual_bounding_rect().center())
                }
                _ => None,
            })
            .expect("collapsed section toggle is visible");
        for pressed in [true, false] {
            let (_, action, _) = render(
                egui::RawInput {
                    events: vec![
                        egui::Event::PointerMoved(toggle),
                        egui::Event::PointerButton {
                            pos: toggle,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    ..Default::default()
                },
                true,
            );
            if !pressed {
                assert!(matches!(
                    action,
                    Some(SidebarAction::ToggleNetworkLocations)
                ));
            }
        }
        assert!(tree.is_expanded(Path::new(&locations[0].path)));
    }

    #[test]
    fn network_location_selection_is_case_insensitive_and_respects_path_boundaries() {
        assert!(path_is_at_or_under(
            r"\\SERVER\share\folder",
            r"\\server\share"
        ));
        assert!(path_is_at_or_under(
            r"\\?\UNC\server\share\",
            r"\\server\share"
        ));
        assert!(!path_is_at_or_under(
            r"\\server\share-copy",
            r"\\server\share"
        ));
    }
}
