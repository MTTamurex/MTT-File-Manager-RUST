use super::{SidebarAction, SidebarContext};
use eframe::egui::{self, Color32, Pos2, Sense};
use rust_i18n::t;

pub(super) fn render(
    ui: &mut egui::Ui,
    ctx: &SidebarContext<'_>,
    action: &mut Option<SidebarAction>,
    group: &crate::domain::folder_group::FolderGroup,
    index: usize,
    drag_id: egui::Id,
    is_drag_source: bool,
) -> (egui::Rect, egui::Response) {
    let (mut rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), 28.0),
        Sense::click_and_drag(),
    );
    rect.min.x = ui.clip_rect().min.x;
    rect.max.x = ui.clip_rect().max.x;
    let response = response.on_hover_ui(|ui| {
        crate::ui::folder_group_tooltip::render(ui, group, ctx.tag_definitions);
    });
    if response.drag_started() {
        super::quick_access_items::store_drag_source(ui, rect, index, drag_id);
    }

    if super::quick_access_items::should_paint_item(ui, rect, is_drag_source) {
        if response.hovered() && !ctx.is_item_dragging {
            ui.painter()
                .rect_filled(rect, 0.0, crate::ui::theme::selection_hover_color(ui.ctx()));
        }
        ui.painter().text(
            Pos2::new(rect.left() + 12.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            "▤",
            egui::FontId::proportional(14.0),
            Color32::from_gray(130),
        );
        ui.painter().text(
            Pos2::new(rect.left() + 34.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            &group.name,
            egui::FontId::proportional(11.5),
            ui.visuals().text_color(),
        );
    }

    if response.clicked() && !ctx.is_renaming && !ctx.is_item_dragging {
        *action = Some(SidebarAction::OpenFolderGroup(group.id));
    }
    response.clone().context_menu(|ui| {
        if ui
            .add_enabled(
                !ctx.is_renaming && !ctx.is_item_dragging,
                egui::Button::new(t!("sidebar.group_open_all")),
            )
            .clicked()
        {
            *action = Some(SidebarAction::OpenFolderGroup(group.id));
            ui.close();
        }
        ui.separator();
        if ui
            .add_enabled(
                !ctx.is_renaming && !ctx.is_item_dragging,
                egui::Button::new(t!("sidebar.group_delete")),
            )
            .clicked()
        {
            *action = Some(SidebarAction::DeleteFolderGroup(group.id));
            ui.close();
        }
    });
    ui.add_space(2.0);
    (rect, response)
}
