//! Keyboard navigation - shared logic for list and grid views
//!
//! This module handles keyboard navigation (Arrow keys, Page Up/Down, Enter)
//! in a way that can be reused by both list_view and grid_view.

use crate::app::shortcuts::ShortcutBinding;
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
    reserved_enter_binding: Option<ShortcutBinding>,
) -> KeyboardNavResult {
    // Do not capture keys when a text field (address, search) has focus
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

    let enter_pressed = ui.input(|i| {
        if !i.key_pressed(egui::Key::Enter) {
            return false;
        }

        let current_binding = ShortcutBinding::from_modifiers(egui::Key::Enter, i.modifiers);
        Some(current_binding) != reserved_enter_binding
    });

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
    reserved_enter_binding: Option<ShortcutBinding>,
) -> KeyboardNavResult {
    // Do not capture keys when a text field (address, search) has focus
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

    let enter_pressed = ui.input(|i| {
        if !i.key_pressed(egui::Key::Enter) {
            return false;
        }

        let current_binding = ShortcutBinding::from_modifiers(egui::Key::Enter, i.modifiers);
        Some(current_binding) != reserved_enter_binding
    });

    KeyboardNavResult {
        new_index,
        page_action,
        enter_pressed,
    }
}

pub fn process_column_list_keyboard_input(
    ui: &egui::Ui,
    current_index: Option<usize>,
    item_count: usize,
    rows_per_column: usize,
    visible_columns: usize,
    reserved_enter_binding: Option<ShortcutBinding>,
) -> KeyboardNavResult {
    if ui.ctx().wants_keyboard_input() || item_count == 0 {
        return KeyboardNavResult::no_action();
    }

    let direction = if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
        Some(ColumnDirection::Down)
    } else if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
        Some(ColumnDirection::Up)
    } else if ui.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
        Some(ColumnDirection::Right)
    } else if ui.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
        Some(ColumnDirection::Left)
    } else if ui.input(|i| i.key_pressed(egui::Key::PageDown)) {
        Some(ColumnDirection::PageDown)
    } else if ui.input(|i| i.key_pressed(egui::Key::PageUp)) {
        Some(ColumnDirection::PageUp)
    } else {
        None
    };

    let new_index = direction.map(|direction| {
        current_index.map_or(0, |current| {
            calculate_column_list_index(
                current,
                item_count,
                rows_per_column,
                visible_columns,
                direction,
            )
        })
    });
    let enter_pressed = ui.input(|i| {
        i.key_pressed(egui::Key::Enter)
            && Some(ShortcutBinding::from_modifiers(
                egui::Key::Enter,
                i.modifiers,
            )) != reserved_enter_binding
    });

    KeyboardNavResult {
        new_index,
        page_action: None,
        enter_pressed,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnDirection {
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
}

pub fn calculate_column_list_index(
    current_index: usize,
    item_count: usize,
    rows_per_column: usize,
    visible_columns: usize,
    direction: ColumnDirection,
) -> usize {
    if item_count == 0 || rows_per_column == 0 {
        return 0;
    }
    let current = current_index.min(item_count - 1);
    let row = current % rows_per_column;
    let page_jump = rows_per_column * visible_columns.max(1);

    match direction {
        ColumnDirection::Up if row > 0 => current - 1,
        ColumnDirection::Down if row + 1 < rows_per_column && current + 1 < item_count => {
            current + 1
        }
        ColumnDirection::Left => current.saturating_sub(rows_per_column),
        ColumnDirection::Right => (current + rows_per_column).min(item_count - 1),
        ColumnDirection::PageUp => current.saturating_sub(page_jump),
        ColumnDirection::PageDown => (current + page_jump).min(item_count - 1),
        _ => current,
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

#[cfg(test)]
mod tests {
    use super::{calculate_column_list_index, ColumnDirection};

    #[test]
    fn column_navigation_uses_column_major_order() {
        assert_eq!(
            calculate_column_list_index(4, 10, 3, 1, ColumnDirection::Up),
            3
        );
        assert_eq!(
            calculate_column_list_index(4, 10, 3, 1, ColumnDirection::Down),
            5
        );
        assert_eq!(
            calculate_column_list_index(4, 10, 3, 1, ColumnDirection::Left),
            1
        );
        assert_eq!(
            calculate_column_list_index(4, 10, 3, 1, ColumnDirection::Right),
            7
        );
    }

    #[test]
    fn right_navigation_clamps_in_incomplete_last_column() {
        assert_eq!(
            calculate_column_list_index(8, 10, 3, 1, ColumnDirection::Right),
            9
        );
        assert_eq!(
            calculate_column_list_index(9, 10, 3, 1, ColumnDirection::Down),
            9
        );
    }

    #[test]
    fn page_navigation_moves_by_visible_columns() {
        assert_eq!(
            calculate_column_list_index(2, 20, 4, 2, ColumnDirection::PageDown),
            10
        );
        assert_eq!(
            calculate_column_list_index(10, 20, 4, 2, ColumnDirection::PageUp),
            2
        );
    }

    #[test]
    fn first_navigation_target_is_first_item() {
        let current_index = None;
        let target = current_index.map_or(0, |current| {
            calculate_column_list_index(current, 20, 4, 2, ColumnDirection::Down)
        });
        assert_eq!(target, 0);
    }
}
