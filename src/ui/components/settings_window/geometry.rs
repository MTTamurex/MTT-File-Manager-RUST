use eframe::egui;
use rust_i18n::t;

fn available_size(ctx: &egui::Context) -> egui::Vec2 {
    (ctx.viewport_rect().size() - egui::vec2(24.0, 24.0)).max(egui::vec2(1.0, 1.0))
}

pub(super) fn window(ctx: &egui::Context) -> egui::Window<'static> {
    let id = egui::Id::new("settings_window");
    let center = ctx.viewport_rect().center();
    let window = egui::Window::new(t!("settings.window_title").to_string())
        .id(id)
        .title_bar(false)
        .collapsible(false)
        .resizable(true)
        .default_width(912.0)
        .default_height(740.0)
        .min_size(egui::Vec2::ZERO)
        .max_size(available_size(ctx))
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(center)
        .order(egui::Order::Foreground);

    let was_visible = ctx.memory(|memory| {
        memory
            .areas()
            .visible_last_frame(&egui::LayerId::new(egui::Order::Foreground, id))
    });
    if was_visible {
        window
    } else {
        // Center only when opening. A permanent anchor moves the opposite edge
        // during resizing and makes the dragged edge drift away from the pointer.
        window.movable(false).current_pos(center)
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
