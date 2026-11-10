//! The modal is rendered before file views; its command is dispatched after them.
mod description;
mod keyboard;
mod layout;
mod list;
mod row;
mod selection;
mod tooltip;
pub(crate) use keyboard::consume_toggle;
#[cfg(test)]
mod description_tests;
#[cfg(test)]
mod layout_tests;
#[cfg(test)]
mod selection_tests;
#[cfg(test)]
mod tests;

#[cfg(test)]
use crate::app::command_palette::input_hints;
use crate::app::command_palette::{
    availability::CommandContext, catalog::Command, search, CommandPaletteState,
};
use crate::app::{shortcuts::ShortcutBindings, ImageViewerApp};
use eframe::egui;
#[cfg(test)]
use rust_i18n::t;

pub(crate) fn input_id() -> egui::Id {
    egui::Id::new("command_palette_query")
}

pub(crate) fn render(app: &mut ImageViewerApp, ctx: &egui::Context) -> Option<Command> {
    // An asynchronous notification can open a higher-priority dialog while we are open.
    if app.command_palette_blocked() {
        app.command_palette.close(ctx);
        return None;
    }
    let context = CommandContext::from_app(app);
    render_dialog(ctx, &mut app.command_palette, &context, &app.shortcuts)
}

fn render_dialog(
    ctx: &egui::Context,
    state: &mut CommandPaletteState,
    context: &CommandContext,
    shortcuts: &ShortcutBindings,
) -> Option<Command> {
    let mut execute = None;
    let close = consume_toggle(shortcuts, ctx);
    let keys = keyboard::read(ctx);
    let pointer_moved = selection::pointer_moved(state, ctx);
    let response = egui::Modal::new(egui::Id::new("command_palette_modal"))
        .area(
            egui::Modal::default_area(egui::Id::new("command_palette_modal"))
                .constrain_to(ctx.viewport_rect()),
        )
        .frame(
            egui::Frame::window(&ctx.global_style())
                .inner_margin(16.0)
                .corner_radius(12.0),
        )
        .show(ctx, |ui| {
            let columns = layout::allocate(ui, shortcuts);
            let mut left = layout::column(ui, columns.list, "command_palette_left");
            execute = list::render(&mut left, state, context, shortcuts, &keys, pointer_moved);
            let described = search::entries(&state.query, shortcuts)
                .get(state.selected)
                .map(|entry| entry.command);
            let mut right = layout::column(ui, columns.description, "command_palette_description");
            description::render(&mut right, described, shortcuts);
        });
    let close = close || keys.close || response.should_close();
    if close || execute.is_some() {
        state.close(ctx);
        // Ignore a coincident Enter/click when Escape or the backdrop closes the palette.
        if close {
            execute = None;
        }
    }
    execute
}

/// Block raw keyboard handlers as well as widgets, including the closing frame.
pub(crate) fn block_background_input(ctx: &egui::Context) {
    ctx.input_mut(|input| {
        input.events.retain(|event| {
            !matches!(
                event,
                egui::Event::Key { .. }
                    | egui::Event::Text(_)
                    | egui::Event::Copy
                    | egui::Event::Cut
                    | egui::Event::Paste(_)
                    | egui::Event::MouseWheel { .. }
            )
        });
        input.smooth_scroll_delta = egui::Vec2::ZERO;
        input.raw.dropped_files.clear();
        input.raw.hovered_files.clear();
    });
}
