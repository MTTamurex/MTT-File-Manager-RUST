use crate::app::settings_window_geometry::SettingsWindowGeometry;
use eframe::egui;
use rust_i18n::t;

fn available_size(ctx: &egui::Context) -> egui::Vec2 {
    (ctx.viewport_rect().size() - egui::vec2(24.0, 24.0)).max(egui::vec2(1.0, 1.0))
}

pub(super) fn window(
    ctx: &egui::Context,
    saved: Option<SettingsWindowGeometry>,
) -> egui::Window<'static> {
    let id = egui::Id::new("settings_window");
    let viewport = ctx.viewport_rect();
    let rect = saved.map(|geometry| geometry.rect()).unwrap_or_else(|| {
        egui::Rect::from_center_size(viewport.center(), egui::vec2(912.0, 740.0))
    });
    let size = rect.size().min(available_size(ctx));
    let min = if saved.is_some() {
        rect.min.clamp(viewport.min, viewport.max - size)
    } else {
        viewport.center() - size * 0.5
    };
    let rect = egui::Rect::from_min_size(min, size);
    let window = egui::Window::new(t!("settings.window_title").to_string())
        .id(id)
        .title_bar(false)
        .collapsible(false)
        .resizable(true)
        .default_size(rect.size())
        .min_size(egui::Vec2::ZERO)
        .max_size(available_size(ctx))
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(rect.center())
        .order(egui::Order::Foreground);

    let was_visible = ctx.memory(|memory| {
        memory
            .areas()
            .visible_last_frame(&egui::LayerId::new(egui::Order::Foreground, id))
    });
    if was_visible {
        window
    } else if saved.is_some() || ctx.memory(|memory| memory.area_rect(id).is_none()) {
        // Restore only when opening; forcing a position during interaction
        // makes the opposite edge drift while resizing.
        window.movable(false).current_pos(rect.center())
    } else {
        window
    }
}

pub(super) fn block_content_drag(ui: &egui::Ui) {
    // egui disables left/top resizing on immovable, unanchored windows. Keep
    // the window movable for edge resizing, but absorb background drags before
    // rendering controls so dragging the body does not move the settings window.
    // Call after the header: leave it available for egui's window dragging.
    ui.interact(
        ui.available_rect_before_wrap(),
        ui.id().with("settings_content_drag_blocker"),
        egui::Sense::DRAG,
    );
}

#[cfg(test)]
mod tests;
