use crate::app::shortcuts::{clear_scroll, ShortcutAction, ShortcutBinding, ShortcutBindings};
use crate::app::ImageViewerApp;
use crate::domain::file_entry::ViewMode;
use eframe::egui;

#[derive(Clone, Default)]
struct ScrollAccumulator {
    binding: Option<ShortcutBinding>,
    amount: f32,
    last_event: f64,
}

impl ScrollAccumulator {
    fn add(&mut self, binding: ShortcutBinding, amount: f32, now: f64) -> u32 {
        if self.binding != Some(binding) || now - self.last_event > 0.3 {
            self.amount = 0.0;
        }
        self.binding = Some(binding);
        self.last_event = now;
        self.amount += amount;
        let steps = self.amount.floor();
        self.amount -= steps;
        steps as u32
    }
}

fn wheel_amount(unit: egui::MouseWheelUnit, y: f32) -> f32 {
    match unit {
        egui::MouseWheelUnit::Point => y / 40.0,
        egui::MouseWheelUnit::Line | egui::MouseWheelUnit::Page => y,
    }
}

pub(super) fn handle_view_mode_input(app: &mut ImageViewerApp, ctx: &egui::Context) -> bool {
    if app.context_menu.is_open
        || app.show_tag_manager
        || app.batch_rename_state.is_some()
        || app.is_item_dragging
        || app.file_panel_input_blocked_by_drag_move_confirmation()
    {
        ctx.data_mut(|data| {
            data.remove::<ScrollAccumulator>(egui::Id::new("view_mode_scroll_accumulator"))
        });
        return false;
    }
    let input = read_view_mode_input(&app.shortcuts, ctx, app.view_mode);
    if input.thumbnail_delta != 0.0 {
        app.thumbnail_size = (app.thumbnail_size + input.thumbnail_delta)
            .clamp(crate::ui::theme::THUMBNAIL_MIN, 256.0);
        app.save_preferences();
    }
    app.change_view_mode(input.requested_mode);
    input.activity
}

struct ViewModeInput {
    requested_mode: ViewMode,
    thumbnail_delta: f32,
    activity: bool,
}

fn read_view_mode_input(
    shortcuts: &ShortcutBindings,
    ctx: &egui::Context,
    view_mode: ViewMode,
) -> ViewModeInput {
    let next = shortcuts.get(ShortcutAction::NextViewMode);
    let previous = shortcuts.get(ShortcutAction::PreviousViewMode);
    let state_id = egui::Id::new("view_mode_scroll_accumulator");
    let mut accumulator = ctx.data_mut(|data| {
        data.get_temp::<ScrollAccumulator>(state_id)
            .unwrap_or_default()
    });
    let now = ctx.input(|input| input.time);
    let mut requested_mode = view_mode;
    let mut thumbnail_delta = 0.0;
    let mut consumed_wheel = false;
    let mut activity = false;
    ctx.input_mut(|input| {
        input.events.retain(|event| {
            let egui::Event::MouseWheel {
                unit,
                delta,
                modifiers,
                ..
            } = event
            else {
                return true;
            };
            if delta.y == 0.0 || !delta.y.is_finite() {
                if delta.x != 0.0 {
                    accumulator.binding = None;
                }
                return true;
            }
            let binding = ShortcutBinding::scroll(
                modifiers.ctrl,
                modifiers.alt,
                modifiers.shift,
                delta.y > 0.0,
            );
            let amount = wheel_amount(*unit, delta.y);
            if binding == next || binding == previous {
                consumed_wheel = true;
                activity = true;
                let steps = accumulator.add(binding, amount.abs(), now);
                for _ in 0..steps % 4 {
                    requested_mode = requested_mode.cycle(binding == next);
                }
                false
            } else if modifiers.ctrl
                && modifiers.shift
                && !modifiers.alt
                && view_mode == ViewMode::Grid
            {
                consumed_wheel = true;
                activity = true;
                accumulator.binding = Some(binding);
                accumulator.amount = 0.0;
                accumulator.last_event = now;
                thumbnail_delta += amount * 4.0;
                false
            } else {
                accumulator.binding = None;
                true
            }
        });
        // Suppress the smooth tail of a consumed wheel gesture as well.
        if consumed_wheel || (accumulator.binding.is_some() && now - accumulator.last_event < 0.3) {
            clear_scroll(input);
        }
    });
    ctx.data_mut(|data| data.insert_temp(state_id, accumulator));
    if !next.is_scroll() && shortcuts.consume_triggered(ShortcutAction::NextViewMode, ctx) {
        requested_mode = requested_mode.cycle(true);
        activity = true;
    }
    if !previous.is_scroll() && shortcuts.consume_triggered(ShortcutAction::PreviousViewMode, ctx) {
        requested_mode = requested_mode.cycle(false);
        activity = true;
    }
    if consumed_wheel {
        ctx.set_zoom_factor(1.0);
    }
    ViewModeInput {
        requested_mode,
        thumbnail_delta,
        activity,
    }
}

#[cfg(test)]
mod tests;
