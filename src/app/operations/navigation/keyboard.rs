//! Keyboard navigation - shared logic for list and grid views
//!
//! This module handles keyboard navigation (Arrow keys, Page Up/Down, Enter)
//! in a way that can be reused by both list_view and grid_view.

use eframe::egui;

/// View type for keyboard navigation
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ViewType {
    List,
    Grid { cols: usize },
}

/// Navigation action returned by keyboard handlers
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NavigationAction {
    MoveTo(usize),
    PageUp,
    PageDown,
    Enter,
    None,
}

/// Result of keyboard navigation processing
#[derive(Clone, Debug)]
pub struct KeyboardNavResult {
    pub new_index: Option<usize>,
    pub page_action: Option<bool>, // Some(true) = PageDown, Some(false) = PageUp
    pub enter_pressed: bool,
}

impl KeyboardNavResult {
    pub fn no_action() -> Self {
        Self {
            new_index: None,
            page_action: None,
            enter_pressed: false,
        }
    }
}

/// Check if keyboard navigation should be active
///
/// Navigation is disabled when renaming or when media has keyboard focus
pub fn should_handle_navigation(_ui: &egui::Ui, is_renaming: bool, is_media_focused: bool) -> bool {
    !is_renaming && !is_media_focused
}

/// Process keyboard input for list view navigation
///
/// Returns the navigation result without modifying any state
pub fn process_list_keyboard_input(
    ui: &egui::Ui,
    current_index: Option<usize>,
    item_count: usize,
    row_height: f32,
    viewport_h: f32,
) -> KeyboardNavResult {
    // Não capturar teclas quando um campo de texto (endereço, busca) está com foco
    if ui.ctx().wants_keyboard_input() {
        return KeyboardNavResult::no_action();
    }

    let mut pending_delta: i32 = 0;
    let mut page_action: Option<bool> = None;

    if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
        pending_delta += 1;
    }
    if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
        pending_delta -= 1;
    }
    if ui.input(|i| i.key_pressed(egui::Key::PageDown)) {
        page_action = Some(true);
    }
    if ui.input(|i| i.key_pressed(egui::Key::PageUp)) {
        page_action = Some(false);
    }

    let visible_count = (viewport_h / row_height).floor() as usize;
    let mut new_index = None;

    if let Some(is_down) = page_action {
        if is_down {
            new_index = Some(
                current_index
                    .map(|idx| (idx + visible_count).min(item_count.saturating_sub(1)))
                    .unwrap_or(visible_count),
            );
        } else {
            // PageUp - simple subtraction from current position
            new_index = Some(
                current_index
                    .map(|idx| idx.saturating_sub(visible_count))
                    .unwrap_or(0),
            );
        }
    } else if pending_delta != 0 {
        new_index = Some(
            current_index
                .map(|idx| {
                    (idx as i32 + pending_delta).clamp(0, item_count.saturating_sub(1) as i32)
                        as usize
                })
                .unwrap_or(0),
        );
    }

    // Alt+Enter é reservado para Propriedades — não dispara "abrir"
    let enter_pressed = ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.alt);

    KeyboardNavResult {
        new_index,
        page_action,
        enter_pressed,
    }
}

/// Process keyboard input for grid view navigation
///
/// Returns the navigation result without modifying any state
pub fn process_grid_keyboard_input(
    ui: &egui::Ui,
    current_index: Option<usize>,
    item_count: usize,
    cols: usize,
    cell_h: f32,
    viewport_h: f32,
) -> KeyboardNavResult {
    // Não capturar teclas quando um campo de texto (endereço, busca) está com foco
    if ui.ctx().wants_keyboard_input() {
        return KeyboardNavResult::no_action();
    }

    let mut pending_delta: i32 = 0;
    let mut page_action: Option<bool> = None;

    if ui.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
        pending_delta += 1;
    }
    if ui.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
        pending_delta -= 1;
    }
    if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
        pending_delta += cols as i32;
    }
    if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
        pending_delta -= cols as i32;
    }
    if ui.input(|i| i.key_pressed(egui::Key::PageDown)) {
        page_action = Some(true);
    }
    if ui.input(|i| i.key_pressed(egui::Key::PageUp)) {
        page_action = Some(false);
    }

    let visible_rows = (viewport_h / cell_h).floor() as usize;
    let jump = visible_rows * cols;

    let mut new_index = None;
    if let Some(is_down) = page_action {
        if is_down {
            new_index = Some(
                current_index
                    .map(|idx| (idx + jump).min(item_count.saturating_sub(1)))
                    .unwrap_or(jump),
            );
        } else {
            new_index = Some(
                current_index
                    .map(|idx| idx.saturating_sub(jump))
                    .unwrap_or(0),
            );
        }
    } else if pending_delta != 0 {
        new_index = Some(
            current_index
                .map(|idx| {
                    (idx as i32 + pending_delta).clamp(0, item_count.saturating_sub(1) as i32)
                        as usize
                })
                .unwrap_or(0),
        );
    }

    // Alt+Enter é reservado para Propriedades — não dispara "abrir"
    let enter_pressed = ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.alt);

    KeyboardNavResult {
        new_index,
        page_action,
        enter_pressed,
    }
}

/// Calculate the new index based on current position and desired movement
///
/// This is a helper that can be used by both views
pub fn calculate_new_index(current_index: Option<usize>, delta: i32, item_count: usize) -> usize {
    current_index
        .map(|idx| (idx as i32 + delta).clamp(0, item_count.saturating_sub(1) as i32) as usize)
        .unwrap_or(0)
}

/// Clamp index to valid bounds
pub fn clamp_index(index: usize, item_count: usize) -> usize {
    index.min(item_count.saturating_sub(1))
}
