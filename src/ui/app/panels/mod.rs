use crate::app::ImageViewerApp;
use eframe::egui;

mod content;
mod folder_error;
mod panel_tab_bar;
mod sidebar_actions;
mod sidebar_layout;
mod sidebar_panel;

// Sidebar width constraints
const LEFT_SIDEBAR_MIN: f32 = 150.0;
const LEFT_SIDEBAR_MAX: f32 = 500.0;
const RIGHT_SIDEBAR_MIN: f32 = 250.0;
const RIGHT_SIDEBAR_MAX: f32 = 500.0;
const RESIZE_HANDLE_WIDTH: f32 = 6.0;

pub fn render_panels(app: &mut ImageViewerApp, root_ui: &mut egui::Ui, frame: &mut eframe::Frame) {
    let ctx = root_ui.ctx().clone();
    let t_panels_start = std::time::Instant::now();

    // Resize handles render first so foreground windows stay on top.
    let t_resize = std::time::Instant::now();
    render_resize_handles(app, &ctx);
    let resize_ms = t_resize.elapsed().as_millis();

    let t_sidebar = std::time::Instant::now();
    let sidebar_action = sidebar_panel::render(app, root_ui);
    let sidebar_ms = t_sidebar.elapsed().as_millis();
    // Navigation I/O is deliberately excluded from sidebar render timing.
    if let Some(action) = sidebar_action {
        let _diagnostic_stage =
            crate::infrastructure::diagnostic_logger::navigation_trace::Stage::new(
                "sidebar_action",
            );
        sidebar_actions::handle(app, action);
    }

    let t_preview = std::time::Instant::now();
    let mut diagnostic_stage =
        crate::infrastructure::diagnostic_logger::navigation_trace::Stage::new("panel_preview");
    content::render_preview_panel_layout(app, root_ui, frame);
    let preview_ms = t_preview.elapsed().as_millis();

    let t_central = std::time::Instant::now();
    diagnostic_stage.lap("panel_central");
    content::render_central_panel_layout(app, root_ui);
    if let Some(request) = crate::ui::new_tab_request::take(&ctx) {
        if !app.tab_navigation_blocked() {
            app.open_locations_in_new_tabs(request.panel, &[request.path], request.shift);
        }
    }
    app.process_pending_pinned_navigation();
    app.process_pending_archive_activations();
    let central_ms = t_central.elapsed().as_millis();
    diagnostic_stage.lap("panel_focus_release");

    // Release focus back to the main window when clicking outside the player.
    #[cfg(target_os = "windows")]
    {
        use crate::ui::components::MediaPreview;
        if ctx.input(|i| i.pointer.any_pressed()) {
            if let Some(MediaPreview::Video(ref player)) = app.media_preview {
                player.release_focus_auto();
            }
        }
    }

    let total_ms = t_panels_start.elapsed().as_millis();
    if total_ms > 120 {
        let items_len = app.items.len();
        let all_items_len = app.all_items.len();
        let pending_thumbs = app.pending_thumbnails.len();
        let loading_icons = app.loading_icons.len();
        log::warn!(
            "[PERF] Slow render_panels breakdown: resize={}ms sidebar={}ms preview={}ms central={}ms total={}ms | view={:?} items={} all_items={} loading_folder={} pending_thumbs={} loading_icons={}",
            resize_ms,
            sidebar_ms,
            preview_ms,
            central_ms,
            total_ms,
            app.view_mode,
            items_len,
            all_items_len,
            app.is_loading_folder,
            pending_thumbs,
            loading_icons,
        );
    }
}

fn render_resize_handles(app: &mut ImageViewerApp, ctx: &egui::Context) {
    // Prevent click-through while the settings modal is active.
    if app.navigation_state.show_settings_window {
        return;
    }
    let screen = ctx.viewport_rect();
    let top_panels_height = 36.0
        + 46.0
        + if crate::ui::app::layers::quick_access_bar_visible(app) {
            crate::ui::app::layers::QUICK_ACCESS_BAR_HEIGHT
        } else {
            0.0
        }
        + 46.0;

    if app.show_left_sidebar {
        let left_width = app
            .layout
            .sidebar_left_width
            .clamp(LEFT_SIDEBAR_MIN, LEFT_SIDEBAR_MAX);
        let left_handle_rect = egui::Rect::from_min_size(
            egui::pos2(left_width - RESIZE_HANDLE_WIDTH / 2.0, top_panels_height),
            egui::vec2(RESIZE_HANDLE_WIDTH, screen.height() - top_panels_height),
        );
        egui::Area::new(egui::Id::new("left_sidebar_resize"))
            .fixed_pos(left_handle_rect.min)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let response = ui.allocate_rect(left_handle_rect, egui::Sense::drag());
                if response.hovered() || response.dragged() {
                    ctx.set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                }
                if response.dragged() {
                    let delta = response.drag_delta().x;
                    app.layout.sidebar_left_width = (app.layout.sidebar_left_width + delta)
                        .clamp(LEFT_SIDEBAR_MIN, LEFT_SIDEBAR_MAX);
                }
            });
    }
    if app.show_preview_panel {
        let right_width = app
            .layout
            .sidebar_right_width
            .clamp(RIGHT_SIDEBAR_MIN, RIGHT_SIDEBAR_MAX);
        let right_handle_x = screen.width() - right_width - RESIZE_HANDLE_WIDTH / 2.0;
        let right_handle_rect = egui::Rect::from_min_size(
            egui::pos2(right_handle_x, top_panels_height),
            egui::vec2(RESIZE_HANDLE_WIDTH, screen.height() - top_panels_height),
        );
        egui::Area::new(egui::Id::new("right_sidebar_resize"))
            .fixed_pos(right_handle_rect.min)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let response = ui.allocate_rect(right_handle_rect, egui::Sense::drag());
                if response.hovered() || response.dragged() {
                    ctx.set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                }
                if response.dragged() {
                    let delta = -response.drag_delta().x;
                    app.layout.sidebar_right_width = (app.layout.sidebar_right_width + delta)
                        .clamp(RIGHT_SIDEBAR_MIN, RIGHT_SIDEBAR_MAX);
                }
            });
    }
}
