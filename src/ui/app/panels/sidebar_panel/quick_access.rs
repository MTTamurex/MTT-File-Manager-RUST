use super::{context, sidebar_layout};
use crate::app::navigation_state::QuickAccessPlacement;
use crate::app::ImageViewerApp;
use crate::ui::sidebar::{render_quick_access, SidebarAction};
use eframe::egui;

pub(super) struct Output {
    pub action: Option<SidebarAction>,
    pub interaction_rect: egui::Rect,
    pub height_changed: bool,
}

pub(super) fn render(app: &mut ImageViewerApp, ui: &mut egui::Ui, can_drop: bool) -> Output {
    let visible =
        app.show_quick_access && app.quick_access_placement == QuickAccessPlacement::Sidebar;
    let content_height = if visible {
        sidebar_layout::quick_access_content_height(
            app.collapse_quick_access,
            app.pinned_folders.len(),
            app.show_recycle_bin,
            ui.spacing().item_spacing.y,
        )
    } else {
        0.0
    };
    let max_height =
        ((ui.available_height() - sidebar_layout::QUICK_ACCESS_DIVIDER_HEIGHT) * 0.75).max(0.0);
    let min_height = sidebar_layout::QUICK_ACCESS_MIN_HEIGHT.min(max_height);
    let height = if visible && app.collapse_quick_access {
        content_height.clamp(min_height, max_height)
    } else if visible {
        sidebar_layout::section_height(
            content_height,
            app.layout.sidebar_quick_access_height,
            min_height,
            max_height,
        )
    } else {
        0.0
    };
    let mut result = Output {
        action: None,
        interaction_rect: egui::Rect::NOTHING,
        height_changed: false,
    };
    if height <= 0.0 {
        return result;
    }
    let output = ui
        .scope(|ui| {
            ui.spacing_mut().scroll.fade.strength = 0.0;
            egui::ScrollArea::vertical()
                .id_salt("sidebar_quick_access_scroll")
                .auto_shrink([false, true])
                .max_height(height)
                .min_scrolled_height(0.0)
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    render_quick_access(ui, &mut context(app, can_drop))
                })
        })
        .inner;
    result.action = output.inner;
    result.interaction_rect = output.inner_rect;

    let (mut rect, _) = ui.allocate_exact_size(
        egui::vec2(
            ui.available_width(),
            sidebar_layout::QUICK_ACCESS_DIVIDER_HEIGHT,
        ),
        egui::Sense::hover(),
    );
    rect.min.x = ui.clip_rect().min.x;
    rect.max.x = ui.clip_rect().max.x;
    let sense = if app.collapse_quick_access {
        egui::Sense::hover()
    } else {
        egui::Sense::click_and_drag()
    };
    let divider = ui.interact(
        rect,
        egui::Id::new("sidebar_quick_access_divider_drag"),
        sense,
    );
    if !app.collapse_quick_access && (divider.hovered() || divider.dragged()) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }
    if divider.dragged() {
        app.layout.sidebar_quick_access_height =
            Some((height + divider.drag_delta().y).clamp(min_height, max_height));
        ui.ctx().request_repaint();
    }
    result.height_changed = divider.drag_stopped();
    if divider.double_clicked() {
        app.layout.sidebar_quick_access_height = None;
        result.height_changed = true;
    }
    if ui.is_rect_visible(rect) {
        let stroke = if divider.hovered() || divider.dragged() {
            egui::Stroke::new(1.5, ui.visuals().widgets.active.bg_fill)
        } else {
            ui.visuals().widgets.noninteractive.bg_stroke
        };
        ui.painter()
            .hline(rect.min.x..=rect.max.x, rect.center().y, stroke);
    }
    result.interaction_rect = result.interaction_rect.union(rect);
    result
}
