use super::SidebarAction;
use eframe::egui;
use rust_i18n::t;

pub(crate) fn menu(response: &egui::Response, blocked: bool) -> Option<SidebarAction> {
    if blocked {
        return None;
    }
    let mut action = None;
    response.context_menu(|ui| {
        for (collapsed, text) in [
            (true, t!("sidebar.collapse_all")),
            (false, t!("sidebar.expand_all")),
        ] {
            if ui.button(text).clicked() {
                action = Some(SidebarAction::SetAllSectionsCollapsed(collapsed));
                ui.close();
            }
        }
    });
    action
}

pub(crate) fn header_menu(
    ui: &egui::Ui,
    rect: egui::Rect,
    id: &str,
    blocked: bool,
) -> Option<SidebarAction> {
    let response = ui.interact(
        rect,
        ui.id().with(("section_menu", id)),
        egui::Sense::click(),
    );
    menu(&response, blocked)
}

pub(crate) fn apply(app: &mut crate::app::ImageViewerApp, collapsed: bool) {
    app.layout.set_sidebar_sections_collapsed(collapsed);
    app.collapse_quick_access = collapsed;
    app.collapse_cloud_drives = collapsed;
    app.collapse_local_disks = collapsed;
    app.collapse_network_drives = collapsed;
    app.collapse_network_locations = collapsed;
    app.collapse_tags = collapsed;
    // Quick Access already fits collapsed headers without changing its height.
    // Keep that override and the remembered per-section divider limits intact.
    app.sidebar_tree.scroll_target_y = 0.0;
    app.sidebar_tree.scroll_visual_y = 0.0;
    app.sync_to_tab();
    app.save_preferences();
    app.ui_ctx.request_repaint();
}
