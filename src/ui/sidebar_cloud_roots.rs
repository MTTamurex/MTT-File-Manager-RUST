use crate::ui::sidebar::{SidebarAction, SidebarContext};
use eframe::egui::{self, Color32, Pos2, Rect, Sense};
use rust_i18n::t;
use std::path::Path;

#[cfg(test)]
mod rename_tests;

pub fn render_cloud_roots(
    ui: &mut egui::Ui,
    ctx: &mut SidebarContext,
    action: &mut Option<SidebarAction>,
    sidebar_viewport_width: f32,
) {
    if ctx.cloud_roots.is_empty() {
        return;
    }

    let (header_rect, _) = crate::ui::sidebar::allocate_sidebar_viewport_row(
        ui,
        sidebar_viewport_width,
        16.0,
        Sense::hover(),
    );
    if let Some(command) = crate::ui::sidebar::section_menu::header_menu(
        ui,
        header_rect,
        "cloud",
        ctx.is_renaming || ctx.is_item_dragging,
    ) {
        *action = Some(command);
    }

    let toggle_size = 18.0;
    let toggle_rect = Rect::from_center_size(
        Pos2::new(
            header_rect.max.x - toggle_size / 2.0 - 3.0,
            header_rect.center().y,
        ),
        egui::vec2(toggle_size, toggle_size),
    );
    let toggle_response = ui
        .interact(
            toggle_rect,
            egui::Id::new("sidebar_toggle_cloud"),
            Sense::click(),
        )
        .on_hover_text(if ctx.collapse_cloud_drives {
            t!("sidebar.expand_section")
        } else {
            t!("sidebar.collapse_section")
        });

    if ui.is_rect_visible(header_rect) {
        ui.painter().text(
            Pos2::new(header_rect.min.x + 8.0, header_rect.center().y),
            egui::Align2::LEFT_CENTER,
            t!("sidebar.cloud_drives"),
            egui::FontId::proportional(10.0),
            Color32::from_gray(120),
        );

        let toggle_icon = if ctx.collapse_cloud_drives { "v" } else { "^" };
        let toggle_color = if toggle_response.hovered() {
            ui.visuals().text_color()
        } else {
            Color32::from_gray(140)
        };
        ui.painter().text(
            toggle_rect.center(),
            egui::Align2::CENTER_CENTER,
            toggle_icon,
            egui::FontId::proportional(14.0),
            toggle_color,
        );
    }

    if toggle_response.clicked() && action.is_none() {
        *action = Some(SidebarAction::ToggleCloudDrives);
    }

    if ctx.collapse_cloud_drives {
        ui.add_space(6.0);
        return;
    }

    ui.add_space(4.0);

    for root in ctx.cloud_roots {
        let root_path = Path::new(&root.path);
        let tree_id = egui::Id::new("cloud_tree").with(&root.path);
        let name = root_path
            .file_name()
            .map(|name| name.to_string_lossy())
            .unwrap_or_else(|| root.label.as_str().into());
        let target =
            crate::application::sidebar_tree::SidebarFolderTarget::new(tree_id, root_path, &name);
        let editing = ctx.folder_rename.filter(|editing| editing.matches(&target));
        let is_expanded = ctx.tree_state.is_expanded(root_path);
        let is_tree_loading = ctx.tree_state.is_loading(root_path);
        let navigation_selected = !ctx.is_computer_view
            && !ctx.is_recycle_bin_view
            && path_starts_with_case_insensitive(ctx.current_path, &root.path);

        let (rect, response) = crate::ui::sidebar::allocate_sidebar_viewport_row(
            ui,
            sidebar_viewport_width,
            28.0,
            Sense::click(),
        );
        crate::ui::sidebar_tree::reveal_row(ui, ctx.tree_state, root_path, root_path, rect);
        let is_selected =
            crate::ui::sidebar_selection::row_is_selected(
                ctx.context_selection,
                root_path,
                Some(target.node_id),
                navigation_selected,
            ) || (response.secondary_clicked() && !ctx.is_renaming && !ctx.is_item_dragging);

        let arrow_zone = Rect::from_min_size(
            Pos2::new(rect.min.x, rect.min.y),
            egui::vec2(20.0, rect.height()),
        );

        if ui.is_rect_visible(rect) {
            let drag_hover = ctx.is_item_dragging
                && ui
                    .input(|inp| inp.pointer.hover_pos())
                    .map(|p| rect.contains(p))
                    .unwrap_or(false);

            if drag_hover {
                ui.painter().rect_stroke(
                    rect,
                    0.0,
                    egui::Stroke::new(2.0, Color32::from_rgb(24, 122, 255)),
                    egui::StrokeKind::Inside,
                );
            } else if is_selected {
                ui.painter()
                    .rect_filled(rect, 0.0, crate::ui::theme::selection_color(ui.ctx()));
            } else if response.hovered() && !ctx.is_item_dragging {
                ui.painter().rect_filled(
                    rect,
                    0.0,
                    crate::ui::theme::selection_hover_color(ui.ctx()),
                );
            }

            let mut cursor_x = rect.min.x + 2.0;
            let arrow_text = if is_tree_loading {
                "..."
            } else if is_expanded {
                "v"
            } else {
                ">"
            };
            let pointer_pos = ui.input(|inp| inp.pointer.hover_pos());
            let arrow_hovered = pointer_pos.map(|p| arrow_zone.contains(p)).unwrap_or(false);
            let arrow_color = if arrow_hovered {
                ui.visuals().text_color()
            } else {
                Color32::from_gray(140)
            };
            ui.painter().text(
                Pos2::new(cursor_x + 8.0, rect.center().y),
                egui::Align2::CENTER_CENTER,
                arrow_text,
                egui::FontId::proportional(10.0),
                arrow_color,
            );
            cursor_x += 18.0;

            let icon = ctx.icon_loader.get_or_load_cloud_root_icon(
                ui.ctx(),
                &root.path,
                root.icon_resource.as_deref(),
            );
            if let Some(icon) = icon {
                let icon_rect = Rect::from_center_size(
                    Pos2::new(cursor_x + 8.0, rect.center().y),
                    egui::vec2(16.0, 16.0),
                );
                ui.painter().image(
                    icon.id(),
                    icon_rect,
                    Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            cursor_x += 24.0;

            if editing.is_none() {
                ui.painter().text(
                    Pos2::new(cursor_x, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    &root.label,
                    egui::FontId::proportional(11.5),
                    if is_selected {
                        crate::ui::theme::selection_text_color(ui.ctx())
                    } else {
                        ui.visuals().text_color()
                    },
                );
            }
        }

        if let Some(editing) = editing {
            let name_rect = Rect::from_min_max(
                Pos2::new(rect.min.x + 44.0, rect.min.y + 2.0),
                Pos2::new(rect.max.x - 4.0, rect.max.y - 2.0),
            );
            if let Some(rename_action) =
                crate::ui::sidebar_tree::rename::render(ui, name_rect, editing)
            {
                *action = Some(rename_action.into());
            }
        }

        if action.is_none()
            && response.middle_clicked()
            && !ctx.is_renaming
            && !ctx.is_item_dragging
        {
            *action = Some(SidebarAction::OpenInNewTab(root.path.clone()));
        } else if action.is_none()
            && (response.double_clicked() || response.clicked())
            && !ctx.is_renaming
            && !ctx.is_item_dragging
        {
            let click_pos = ui.input(|inp| inp.pointer.interact_pos());
            let clicked_arrow = click_pos.map(|p| arrow_zone.contains(p)).unwrap_or(false);

            if response.double_clicked() && !clicked_arrow {
                *action = Some(SidebarAction::TreeToggleExpand(root_path.to_path_buf()));
            } else if response.clicked() {
                if clicked_arrow {
                    *action = Some(SidebarAction::TreeToggleExpand(root_path.to_path_buf()));
                } else {
                    *action = Some(SidebarAction::NavigateTo(root.path.clone()));
                }
            }
        } else if action.is_none() && response.secondary_clicked() && !ctx.is_renaming {
            *action = Some(SidebarAction::OpenTreeContextMenu(target));
        }

        if ctx.is_item_dragging && action.is_none() {
            let pointer_over = ui
                .input(|inp| inp.pointer.hover_pos())
                .map(|p| rect.contains(p))
                .unwrap_or(false);
            let released = ui.input(|inp| inp.pointer.primary_released());

            if released && pointer_over {
                *action = Some(SidebarAction::DropItemsTo(root.path.clone()));
            }
        }

        ui.add_space(2.0);

        if is_expanded {
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
                crate::ui::sidebar_tree::render_drive_tree(ui, &root.path, &mut tree_ctx)
            {
                if action.is_none() {
                    *action = Some(tree_action.into());
                }
            }
        }
    }

    ui.add_space(6.0);
}

fn path_starts_with_case_insensitive(path: &str, prefix: &str) -> bool {
    let path_components: Vec<String> = Path::new(path)
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_lowercase())
        .collect();
    let prefix_components: Vec<String> = Path::new(prefix)
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_lowercase())
        .collect();

    prefix_components.len() <= path_components.len()
        && prefix_components
            .iter()
            .zip(path_components.iter())
            .all(|(prefix, path)| prefix == path)
}
