//! A non-activating, mouse-transparent native viewport for a picked-up tab.
use crate::ui::tab_bar::detach::NativePointer;
use eframe::egui::{self, layers::ShapeIdx};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_ESCAPE};
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

mod native_frame;

// Transparent space only: enough room for the tab-shaped shadow, not a surrounding card.
const PADDING: egui::Vec2 = egui::vec2(10.0, 10.0);

fn viewport_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("floating_tab_drag_preview")
}

fn active_pass_id() -> egui::Id {
    egui::Id::new("floating_tab_preview_active_pass")
}

pub(super) fn finish_frame(ctx: &egui::Context) {
    let pass = ctx.cumulative_pass_nr();
    if ctx.data(|data| data.get_temp::<u64>(active_pass_id())) == Some(pass) {
        // Widgets under a floating tab must not replace the arrow with a text/resize cursor.
        ctx.set_cursor_icon(egui::CursorIcon::Default);
    }
}

pub(super) fn show(
    ui: &egui::Ui,
    (rect, start, end): (egui::Rect, ShapeIdx, ShapeIdx),
    pointer: Option<NativePointer>,
) -> bool {
    let Some(pointer) = pointer.filter(|pointer| pointer.down) else {
        return false;
    };
    // Never fall back to an interactive embedded egui window.
    if ui.ctx().embed_viewports()
        || ui.input(|input| {
            input.key_pressed(egui::Key::Escape)
                || !input.focused
                || input.pointer.primary_released()
        })
    {
        return false;
    }
    let size = preview_size(rect.size());
    let grab = ui
        .input(|input| input.pointer.press_origin())
        .unwrap_or(rect.center())
        - rect.min;
    let grab = egui::vec2(
        grab.x.clamp(0.0, rect.width()),
        grab.y.clamp(0.0, rect.height()),
    );
    let id = viewport_id();
    let native_scale = ui.input(|input| {
        input
            .raw
            .viewports
            .get(&id)
            .and_then(|viewport| viewport.native_pixels_per_point)
    });
    let child_scale = native_scale.map_or(ui.ctx().pixels_per_point(), |scale| {
        scale * ui.ctx().zoom_factor()
    });
    let builder = builder(size, position(pointer.screen_px, child_scale, grab));
    let mut shapes: Vec<egui::Shape> = ui.ctx().graphics(|graphics| {
        graphics
            .get(ui.layer_id())
            .map(|list| {
                list.all_entries()
                    .skip(start.0)
                    .take(end.0 - start.0)
                    .map(|entry| entry.shape.clone())
                    .collect()
            })
            .unwrap_or_default()
    });
    for shape in &mut shapes {
        shape.translate(PADDING - rect.min.to_vec2());
    }
    ui.ctx()
        .show_viewport_deferred(id, builder, move |ui, _class| {
            native_frame::suppress();
            // Child repaints can outlive a parent UI pass (occlusion, Alt+Tab, minimize).
            // Hide immediately on native cancellation/release, without taking capture.
            let current = pointer
                .owner
                .and_then(|owner| {
                    crate::ui::tab_bar::detach::sample_native_pointer(HWND(owner as *mut _))
                })
                .unwrap_or(pointer);
            if !native_drag_active(current) {
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::Visible(false));
                ui.ctx().request_repaint_of(egui::ViewportId::ROOT);
                return;
            }
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::OuterPosition(position(
                    current.screen_px,
                    ui.ctx().pixels_per_point(),
                    grab,
                )));
            let size = preview_size(rect.size());
            if (ui.max_rect().size() - size).length() > 1.0 {
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
            }
            paint_tab(ui, rect.size(), &shapes);
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(16));
        });
    ui.ctx().request_repaint_of(id);
    let pass = ui.ctx().cumulative_pass_nr();
    ui.ctx()
        .data_mut(|data| data.insert_temp(active_pass_id(), pass));
    true
}

fn builder(size: egui::Vec2, position: egui::Pos2) -> egui::ViewportBuilder {
    egui::ViewportBuilder::default()
        .with_title(native_frame::WINDOW_TITLE)
        .with_decorations(false)
        .with_resizable(false)
        .with_active(false)
        .with_taskbar(false)
        .with_mouse_passthrough(true)
        .with_drag_and_drop(false)
        .with_transparent(true)
        .with_has_shadow(false)
        .with_window_level(egui::WindowLevel::AlwaysOnTop)
        .with_inner_size(size)
        .with_position(position)
}

fn position(screen_px: egui::Pos2, pixels_per_point: f32, grab: egui::Vec2) -> egui::Pos2 {
    screen_px / pixels_per_point - grab - PADDING
}

fn preview_size(tab: egui::Vec2) -> egui::Vec2 {
    tab + PADDING * 2.0
}

fn paint_tab(ui: &egui::Ui, size: egui::Vec2, shapes: &[egui::Shape]) {
    let tab = egui::Rect::from_min_size(PADDING.to_pos2(), size);
    let corners = shapes
        .iter()
        .find_map(|shape| match shape {
            egui::Shape::Rect(rect) if rect.rect == tab => Some(rect.corner_radius),
            _ => None,
        })
        .unwrap_or(egui::CornerRadius::same(8));
    ui.painter().add(
        egui::epaint::Shadow {
            offset: [0, 2],
            blur: 8,
            spread: 1,
            color: egui::Color32::from_black_alpha(65),
        }
        .as_shape(tab, corners),
    );
    let painter = ui.painter().with_clip_rect(tab.intersect(ui.max_rect()));
    for shape in shapes {
        painter.add(shape.clone());
    }
}

fn native_drag_active(pointer: NativePointer) -> bool {
    unsafe {
        pointer.down
            && GetAsyncKeyState(VK_ESCAPE.0 as i32) >= 0
            && pointer
                .owner
                .is_none_or(|owner| GetForegroundWindow() == HWND(owner as *mut _))
    }
}

#[cfg(test)]
mod tests;
