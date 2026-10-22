//! Per-tab type-to-search and its precedence over Backspace navigation.
use crate::app::ImageViewerApp;
use eframe::egui;
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_millis(1500);

fn expire(buffer: &mut String, last_input: Instant, now: Instant) {
    if now.saturating_duration_since(last_input) > TIMEOUT {
        buffer.clear();
    }
}

fn consume_backspace(
    ctx: &egui::Context,
    buffer: &mut String,
    last_input: &mut Instant,
    now: Instant,
) -> bool {
    expire(buffer, *last_input, now);
    if ctx.egui_wants_keyboard_input() {
        buffer.clear();
        return false;
    }
    if buffer.is_empty() || !ctx.input(|i| i.viewport().focused.unwrap_or(false)) {
        return false;
    }
    let mut consumed = false;
    ctx.input_mut(|input| {
        input.events.retain(|event| {
            if matches!(event, egui::Event::Key {
                key: egui::Key::Backspace, pressed: true, modifiers, ..
            } if *modifiers == egui::Modifiers::NONE)
            {
                buffer.pop();
                *last_input = now;
                consumed = true;
                return false;
            }
            true
        });
    });
    consumed
}

/// Run before navigation so the last character cannot also trigger GoBack.
pub(super) fn handle_backspace(app: &mut ImageViewerApp, ctx: &egui::Context) -> bool {
    let tab = app.tab_manager.active_mut();
    let consumed = consume_backspace(
        ctx,
        &mut tab.quick_search_buffer,
        &mut tab.quick_search_last_input,
        Instant::now(),
    );
    if consumed {
        perform_quick_search(app);
    }
    consumed
}

pub(super) fn handle_quick_search(app: &mut ImageViewerApp, ctx: &egui::Context) {
    let tab = app.tab_manager.active_mut();
    expire(
        &mut tab.quick_search_buffer,
        tab.quick_search_last_input,
        Instant::now(),
    );
    // Never capture typing inside toolbar search, inline editors or other fields.
    if ctx.egui_wants_keyboard_input() {
        tab.quick_search_buffer.clear();
        return;
    }
    let text = ctx.input(|i| {
        i.events.iter().find_map(|event| {
            if let egui::Event::Text(text) = event {
                let filtered: String = text.chars().filter(|c| !c.is_control()).collect();
                if !filtered.is_empty() {
                    return Some(filtered);
                }
            }
            None
        })
    });
    if let Some(text) = text {
        tab.quick_search_buffer.push_str(&text);
        tab.quick_search_last_input = Instant::now();
        perform_quick_search(app);
    }
}

fn perform_quick_search(app: &mut ImageViewerApp) {
    let query = app.tab_manager.active().quick_search_buffer.to_lowercase();
    if query.is_empty() {
        return;
    }
    if let Some(index) = app
        .items
        .iter()
        .position(|item| item.name.to_lowercase().starts_with(&query))
    {
        app.selected_item = Some(index);
        app.selected_file = Some(app.items[index].clone());
        app.multi_selection.clear();
        app.multi_selection.insert(app.items[index].path.clone());
        app.selection_anchor = Some(index);
        app.scroll_to_selected = true;
        app.last_input = crate::app::state::LastInput::Keyboard;
    }
}

#[cfg(test)]
mod tests;
