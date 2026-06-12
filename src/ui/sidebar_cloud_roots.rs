use crate::ui::sidebar::{SidebarAction, SidebarContext};
use eframe::egui::{self, Color32, Pos2, Rect, Sense};
use rust_i18n::t;
use std::path::Path;

pub fn render_cloud_roots(
    ui: &mut egui::Ui,
    ctx: &mut SidebarContext,
    action: &mut Option<SidebarAction>,
) {
    if ctx.cloud_roots.is_empty() {
        return;
    }

    let (header_rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 16.0), Sense::hover());
    if ui.is_rect_visible(header_rect) {
        ui.painter().text(
            Pos2::new(header_rect.min.x + 8.0, header_rect.center().y),
            egui::Align2::LEFT_CENTER,
            t!("sidebar.cloud_drives"),
            egui::FontId::proportional(10.0),
            Color32::from_gray(120),
        );
    }

    ui.add_space(4.0);

    for root in ctx.cloud_roots {
        let root_path = Path::new(&root.path);
        let is_expanded = ctx.tree_state.is_expanded(root_path);
        let is_tree_loading = ctx.tree_state.is_loading(root_path);
        let is_selected = !ctx.is_computer_view
            && !ctx.is_recycle_bin_view
            && path_starts_with_case_insensitive(ctx.current_path, &root.path);

        let (mut rect, response) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.0), Sense::click());
        rect.min.x = ui.max_rect().min.x;
        rect.max.x = ui.max_rect().max.x;

        let arrow_zone = Rect::from_min_size(
            Pos2::new(rect.min.x, rect.min.y),
            egui::vec2(20.0, rect.height()),
        );

        if ui.is_rect_visible(rect) {
            let dark_mode = ui.visuals().dark_mode;
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
                    .rect_filled(rect, 0.0, crate::ui::theme::selection_color(dark_mode));
            } else if response.hovered() && !ctx.is_item_dragging {
                ui.painter().rect_filled(
                    rect,
                    0.0,
                    crate::ui::theme::selection_hover_color(dark_mode),
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

            ui.painter().text(
                Pos2::new(cursor_x, rect.center().y),
                egui::Align2::LEFT_CENTER,
                &root.label,
                egui::FontId::proportional(11.5),
                if is_selected {
                    crate::ui::theme::selection_text_color(dark_mode)
                } else {
                    ui.visuals().text_color()
                },
            );
        }

        if action.is_none()
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
            *action = Some(SidebarAction::OpenDriveContextMenu(root.path.clone()));
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
            };
            if let Some(tree_action) =
                crate::ui::sidebar_tree::render_drive_tree(ui, &root.path, &mut tree_ctx)
            {
                if action.is_none() {
                    match tree_action {
                        crate::ui::sidebar_tree::SidebarTreeAction::NavigateTo(path) => {
                            *action = Some(SidebarAction::NavigateTo(path));
                        }
                        crate::ui::sidebar_tree::SidebarTreeAction::ToggleExpand(path) => {
                            *action = Some(SidebarAction::TreeToggleExpand(path));
                        }
                        crate::ui::sidebar_tree::SidebarTreeAction::DropItemsTo(path) => {
                            *action = Some(SidebarAction::DropItemsTo(path));
                        }
                    }
                }
            }
        }
    }

    ui.add_space(6.0);
}

fn path_starts_with_case_insensitive(path: &str, prefix: &str) -> bool {
    path.len() >= prefix.len() && path[..prefix.len()].eq_ignore_ascii_case(prefix)
}
