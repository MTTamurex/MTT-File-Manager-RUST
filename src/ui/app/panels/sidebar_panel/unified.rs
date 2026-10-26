use super::context;
use crate::app::navigation_state::QuickAccessPlacement;
use crate::app::ImageViewerApp;
use crate::ui::sidebar::{
    render_quick_access, render_sidebar_drives, render_sidebar_this_pc, render_tags_section,
    SidebarAction, SidebarContext,
};
use eframe::egui;

#[cfg(test)]
mod tests;

fn scroll_area<R>(
    ui: &mut egui::Ui,
    render: impl FnOnce(&mut egui::Ui, f32) -> R,
) -> egui::scroll_area::ScrollAreaOutput<R> {
    ui.spacing_mut().scroll.fade.strength = 0.0;
    egui::ScrollArea::both()
        .id_salt("sidebar_unified_scroll")
        .auto_shrink([false, false])
        .min_scrolled_height(0.0)
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
        .show(ui, |ui| {
            let viewport_width = ui.available_width();
            ui.set_min_width(viewport_width);
            render(ui, viewport_width)
        })
}

pub(super) fn render(
    app: &mut ImageViewerApp,
    ui: &mut egui::Ui,
    can_drop: bool,
) -> Option<SidebarAction> {
    let quick_access_visible =
        app.show_quick_access && app.quick_access_placement == QuickAccessPlacement::Sidebar;
    let mut sidebar_ctx = context(app, can_drop);
    render_content(ui, &mut sidebar_ctx, quick_access_visible).inner
}

fn render_content(
    ui: &mut egui::Ui,
    sidebar_ctx: &mut SidebarContext<'_>,
    quick_access_visible: bool,
) -> egui::scroll_area::ScrollAreaOutput<Option<SidebarAction>> {
    let tags_visible = sidebar_ctx.show_tags && !sidebar_ctx.tag_definitions.is_empty();
    // Native egui scrolling owns this viewport; the independent layout's custom
    // wheel handling and saved divider heights must not run here.
    scroll_area(ui, |ui, viewport_width| {
        let this_pc = render_sidebar_this_pc(ui, sidebar_ctx);
        let quick_access = if quick_access_visible {
            render_quick_access(ui, sidebar_ctx)
        } else {
            None
        };
        let drives = render_sidebar_drives(ui, sidebar_ctx, viewport_width);
        let mut tags = None;
        if tags_visible {
            // A wide folder tree expands its parent's content width. Tags must
            // keep the viewport width so its collapse control remains reachable.
            ui.scope(|ui| {
                ui.set_width(viewport_width);
                ui.separator();
                render_tags_section(ui, sidebar_ctx, &mut tags);
            });
        }
        this_pc.or(quick_access).or(drives).or(tags)
    })
}
