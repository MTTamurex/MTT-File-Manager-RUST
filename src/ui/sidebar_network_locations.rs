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
    if ui.is_rect_visible(header) {
        ui.painter().text(
            Pos2::new(header.min.x + 8.0, header.center().y),
            egui::Align2::LEFT_CENTER,
            t!("sidebar.network_locations"),
            egui::FontId::proportional(10.0),
            Color32::from_gray(120),
        );
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
            let dark = ui.visuals().dark_mode;
            if selected {
                ui.painter()
                    .rect_filled(rect, 0.0, crate::ui::theme::selection_color(dark));
            } else if response.hovered() && !ctx.is_item_dragging {
                ui.painter()
                    .rect_filled(rect, 0.0, crate::ui::theme::selection_hover_color(dark));
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
                        crate::ui::theme::selection_text_color(dark)
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
