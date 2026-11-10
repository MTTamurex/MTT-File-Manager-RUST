//! The modal is rendered before file views; its command is dispatched after them.
mod keyboard;
mod row;
mod selection;
mod tooltip;
pub(crate) use keyboard::consume_toggle;
#[cfg(test)]
mod layout_tests;
#[cfg(test)]
mod selection_tests;
#[cfg(test)]
mod tests;

use crate::app::command_palette::{
    availability::{disabled_reason, CommandContext},
    catalog::Command,
    input_hints, search, CommandPaletteState,
};
use crate::app::{shortcuts::ShortcutBindings, ImageViewerApp};
use eframe::egui;
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
    let mut close = consume_toggle(shortcuts, ctx);
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
            ui.set_width((ctx.viewport_rect().width() - 48.0).clamp(160.0, 640.0));
            ui.heading(t!("command_palette.title"));
            let input = ui.add(
                egui::TextEdit::singleline(&mut state.query)
                    .id(input_id())
                    .hint_text(t!("command_palette.placeholder"))
                    .desired_width(f32::INFINITY),
            );
            if state.request_focus {
                input.request_focus();
                state.request_focus = false;
            }
            if input.changed() {
                state.selected = 0;
                state.reveal_selected = true;
            }
            let entries = search::entries(&state.query, shortcuts);
            state.selected = state.selected.min(entries.len().saturating_sub(1));
            if keys.up {
                state.move_selection(entries.len(), true);
            }
            if keys.down {
                state.move_selection(entries.len(), false);
            }
            close |= keys.close;
            ui.small(t!("command_palette.matches", count = entries.len()));
            ui.separator();
            let mouse_selects =
                pointer_moved && !keys.up && !keys.down && !keys.run && !input.changed();
            let help_height = ui.text_style_height(&egui::TextStyle::Body) * 4.0
                + ui.spacing().item_spacing.y * 2.0;
            let header_height = ui.min_rect().height();
            let results_height = (ctx.viewport_rect().height()
                - header_height
                - help_height
                - ui.text_style_height(&egui::TextStyle::Body)
                - ui.spacing().item_spacing.y * 6.0
                - 56.0)
                .clamp(60.0, 420.0);
            // The theme uses overlay scrollbars. Reserve their fully expanded
            // width locally so hovering the scrollbar never covers shortcut text.
            let scroll = &mut ui.spacing_mut().scroll;
            scroll.floating_allocated_width = scroll.bar_width + 8.0;
            egui::ScrollArea::vertical()
                .id_salt("command_palette_results")
                .max_height(results_height)
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    if entries.is_empty() {
                        ui.label(t!("command_palette.no_matches"));
                    }
                    for (index, entry) in entries.iter().enumerate() {
                        let reason = disabled_reason(entry.command, context);
                        let selected = index == state.selected;
                        let row = row::render(ui, entry, selected, reason.is_none());
                        if mouse_selects && row.contains_pointer() && !row.clicked() {
                            selection::select_hovered(state, index, ctx);
                        }
                        if selected && state.reveal_selected {
                            row.scroll_to_me(Some(egui::Align::Center));
                        }
                        if index == state.selected {
                            let help = input_hints::help(entry.command, shortcuts);
                            let tooltip_text = if let Some(reason) = reason {
                                format!("{}\n\n{help}", t!(reason))
                            } else {
                                help
                            };
                            let _ = tooltip::show(&row, &tooltip_text);
                        }
                        if row.clicked() {
                            state.selected = index;
                            execute = Some(entry.command);
                        }
                    }
                });
            state.reveal_selected = false;
            let described = entries.get(state.selected).map(|entry| entry.command);
            if keys.run {
                if let Some(command) = described {
                    if disabled_reason(command, context).is_none() {
                        execute = Some(command);
                    }
                }
            }
            ui.separator();
            // Reserve a stable help area: changing selection must not recenter the dialog.
            egui::ScrollArea::vertical()
                // Do not inherit another command's position in a long description.
                .id_salt(("command_palette_help", described))
                .max_height(help_height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if let Some(command) = described {
                        if let Some(reason) = disabled_reason(command, context) {
                            ui.add(egui::Label::new(t!(reason)).wrap());
                        }
                        ui.add(egui::Label::new(input_hints::help(command, shortcuts)).wrap());
                    }
                });
            ui.separator();
            ui.label(t!("command_palette.hint"));
        });
    close |= response.should_close();
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
