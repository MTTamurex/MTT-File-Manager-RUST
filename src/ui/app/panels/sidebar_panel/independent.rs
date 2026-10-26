use super::{context, quick_access, sidebar_layout};
use crate::app::ImageViewerApp;
use crate::ui::sidebar::{
    render_sidebar_drives, render_sidebar_this_pc, render_tags_section, SidebarAction,
};
use eframe::egui;

pub(super) fn render(
    app: &mut ImageViewerApp,
    ui: &mut egui::Ui,
    can_drop: bool,
) -> Option<SidebarAction> {
    const SIDEBAR_SCROLL_SPEED: f32 = 5.0;
    let tags_visible = app.show_tags && !app.tag_definitions.is_empty();
    let tags_collapsed = app.collapse_tags;
    let scroll_delta = ui.input(|i| i.smooth_scroll_delta().y);
    let sidebar_rect = ui.max_rect();
    let pointer_in_sidebar = ui.input(|i| {
        i.pointer
            .hover_pos()
            .is_some_and(|p| sidebar_rect.contains(p))
    });

    let this_pc_action = render_sidebar_this_pc(ui, &mut context(app, can_drop));
    let quick_access = quick_access::render(app, ui, can_drop);
    let drives_avail = sidebar_layout::drives_available_height(
        ui.available_height(),
        tags_visible,
        ui.spacing().item_spacing.y,
    );
    let drives_min_h = 24.0_f32.min(drives_avail);
    let drives_scroll_h = app
        .layout
        .sidebar_drives_height
        .unwrap_or(drives_avail)
        .clamp(drives_min_h, drives_avail);
    let pointer_over_quick_access = ui.input(|i| {
        i.pointer
            .hover_pos()
            .is_some_and(|p| quick_access.interaction_rect.contains(p))
    });

    let dt = ui.input(|i| i.predicted_dt).min(0.05);
    let t = (dt * 9.0).min(1.0);
    let diff = app.sidebar_tree.scroll_target_y - app.sidebar_tree.scroll_visual_y;
    if diff.abs() > 1.0 {
        app.sidebar_tree.scroll_visual_y += diff * t;
    } else {
        app.sidebar_tree.scroll_visual_y = app.sidebar_tree.scroll_target_y;
    }
    if (app.sidebar_tree.scroll_visual_y - app.sidebar_tree.scroll_target_y).abs() > 0.5 {
        ui.ctx().request_repaint();
    }
    let scroll_offset = app.sidebar_tree.scroll_visual_y;
    let manually_sized = app.layout.sidebar_drives_height.is_some();
    let mut sidebar_ctx = context(app, can_drop);
    let drives_scope = sidebar_layout::render_drives_block(
        ui,
        drives_scroll_h,
        manually_sized,
        scroll_offset,
        |ui| {
            let viewport_width = ui.available_width();
            ui.set_min_width(viewport_width);
            render_sidebar_drives(ui, &mut sidebar_ctx, viewport_width)
        },
    );
    let drives_rendered_h = drives_scope.response.rect.height();
    let output = drives_scope.inner;

    let tags_output = tags_visible.then(|| {
        sidebar_layout::render_tags_block(
            ui,
            sidebar_layout::TagsBlockLayout {
                collapsed: tags_collapsed,
                drives_height: drives_rendered_h,
                drives_min_height: drives_min_h,
                drives_max_height: drives_avail,
            },
            |ui| {
                let mut action = None;
                render_tags_section(ui, &mut sidebar_ctx, &mut action);
                action
            },
        )
    });
    let mut tags_action = None;
    let mut tags_rect = egui::Rect::NOTHING;
    let mut height_changed = quick_access.height_changed;
    if let Some(tags) = tags_output {
        tags_rect = tags.interaction_rect;
        tags_action = tags.inner;
        if tags.reset_drives_height {
            app.layout.sidebar_drives_height = None;
        } else if let Some(height) = tags.resized_drives_height {
            app.layout.sidebar_drives_height = Some(height);
        }
        height_changed |= tags.height_changed;
    }
    if height_changed {
        app.save_preferences();
    }

    let pointer_over_tags =
        ui.input(|i| i.pointer.hover_pos().is_some_and(|p| tags_rect.contains(p)));
    let wheel_in_drives = scroll_delta != 0.0
        && pointer_in_sidebar
        && !pointer_over_quick_access
        && !pointer_over_tags;
    if wheel_in_drives {
        app.sidebar_tree.scroll_target_y =
            (app.sidebar_tree.scroll_target_y - scroll_delta * SIDEBAR_SCROLL_SPEED).max(0.0);
        ui.ctx().request_repaint();
    }
    let max_scroll = (output.content_size.y - output.inner_rect.height()).max(0.0);
    app.sidebar_tree.scroll_target_y = app.sidebar_tree.scroll_target_y.min(max_scroll);
    app.sidebar_tree.scroll_visual_y = app.sidebar_tree.scroll_visual_y.min(max_scroll);

    // Ignore egui's extra wheel contribution, but accept scrollbar dragging and external scroll requests.
    let actual_offset = output.state.offset.y;
    let drift = actual_offset - scroll_offset;
    if !(wheel_in_drives && drift.abs() > 0.5) && drift.abs() > 2.0 {
        app.sidebar_tree.scroll_target_y = actual_offset;
        app.sidebar_tree.scroll_visual_y = actual_offset;
    }
    this_pc_action
        .or(quick_access.action)
        .or(output.inner)
        .or(tags_action)
}
