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
        let (rect, response) = allocate_sidebar_viewport_row(ui, width, 28.0, Sense::click());
        let selected = !ctx.is_computer_view
            && !ctx.is_recycle_bin_view
            && path_is_at_or_under(ctx.current_path, &location.path);
        if ui.is_rect_visible(rect) {
            let dark = ui.visuals().dark_mode;
            if selected {
                ui.painter()
                    .rect_filled(rect, 0.0, crate::ui::theme::selection_color(dark));
            } else if response.hovered() && !ctx.is_item_dragging {
                ui.painter()
                    .rect_filled(rect, 0.0, crate::ui::theme::selection_hover_color(dark));
            }
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
            let label_rect = Rect::from_min_max(Pos2::new(rect.min.x + 48.0, rect.min.y), rect.max);
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
        if action.is_none() && !ctx.is_renaming && !ctx.is_item_dragging {
            if response.middle_clicked() {
                *action = Some(SidebarAction::OpenInNewTab(location.path.clone()));
            } else if response.clicked() {
                *action = Some(SidebarAction::NavigateTo(location.path.clone()));
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
