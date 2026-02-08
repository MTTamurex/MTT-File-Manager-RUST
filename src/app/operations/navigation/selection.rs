//! Selection logic - shared multi-selection handling for list and grid views
//!
//! This module handles selection logic (Ctrl+Click, Shift+Click, Range selection)
//! in a way that can be reused by both list_view and grid_view.

use std::path::{Path, PathBuf};

use crate::app::state::ImageViewerApp;
use crate::ui::cache::FxHashSet;

/// Selection modifier state
#[derive(Clone, Copy, Debug, Default)]
pub struct SelectionModifiers {
    pub ctrl: bool,
    pub shift: bool,
}

/// Result of a selection operation
#[derive(Clone, Debug)]
pub struct SelectionResult {
    pub selected_item: Option<usize>,
    pub selected_file: Option<PathBuf>,
    pub selection_anchor: Option<usize>,
    pub multi_selection: FxHashSet<PathBuf>,
    pub selection_changed: bool,
}

/// Handle Ctrl+Click selection
///
/// Toggles the item in multi_selection and sets it as the new anchor
pub fn handle_ctrl_click(
    item_index: usize,
    item_path: &Path,
    current_selection: &FxHashSet<PathBuf>,
) -> SelectionResult {
    let mut new_selection = current_selection.clone();

    if new_selection.contains(item_path) {
        new_selection.remove(item_path);
    } else {
        new_selection.insert(item_path.to_path_buf());
    }

    SelectionResult {
        selected_item: Some(item_index),
        selected_file: Some(item_path.to_path_buf()),
        selection_anchor: Some(item_index),
        multi_selection: new_selection,
        selection_changed: true,
    }
}

/// Handle Shift+Click selection (range selection)
///
/// Adds all items between anchor and clicked item to selection
pub fn handle_shift_click(
    item_index: usize,
    item_path: &Path,
    anchor: Option<usize>,
    current_selection: &FxHashSet<PathBuf>,
    get_item_path: impl Fn(usize) -> Option<PathBuf>,
) -> SelectionResult {
    let mut new_selection = current_selection.clone();

    if let Some(anchor_idx) = anchor {
        let (start, end) = if anchor_idx < item_index {
            (anchor_idx, item_index)
        } else {
            (item_index, anchor_idx)
        };

        // Add range to selection (do NOT clear outside selection)
        for i in start..=end {
            if let Some(path) = get_item_path(i) {
                new_selection.insert(path);
            }
        }
    } else {
        // No anchor set - just add this item
        new_selection.insert(item_path.to_path_buf());
    }

    SelectionResult {
        selected_item: Some(item_index),
        selected_file: Some(item_path.to_path_buf()),
        selection_anchor: anchor.or(Some(item_index)),
        multi_selection: new_selection,
        selection_changed: true,
    }
}

/// Handle simple click (no modifiers)
///
/// Clears selection and selects only the clicked item
pub fn handle_simple_click(item_index: usize, item_path: &Path) -> SelectionResult {
    let mut new_selection = FxHashSet::default();
    new_selection.insert(item_path.to_path_buf());

    SelectionResult {
        selected_item: Some(item_index),
        selected_file: Some(item_path.to_path_buf()),
        selection_anchor: Some(item_index),
        multi_selection: new_selection,
        selection_changed: true,
    }
}

/// Handle selection with automatic modifier detection
///
/// This is the main entry point that handles all selection cases
pub fn handle_selection(
    item_index: usize,
    item_path: &Path,
    modifiers: SelectionModifiers,
    anchor: Option<usize>,
    current_selection: &FxHashSet<PathBuf>,
    get_item_path: impl Fn(usize) -> Option<PathBuf>,
) -> SelectionResult {
    if modifiers.ctrl {
        handle_ctrl_click(item_index, item_path, current_selection)
    } else if modifiers.shift {
        handle_shift_click(
            item_index,
            item_path,
            anchor,
            current_selection,
            get_item_path,
        )
    } else {
        handle_simple_click(item_index, item_path)
    }
}

/// Handle keyboard navigation selection with Shift modifier
///
/// This is used when navigating with Shift held down for range selection
pub fn handle_shift_navigation(
    new_index: usize,
    anchor: Option<usize>,
    current_selection: &FxHashSet<PathBuf>,
    get_item_path: impl Fn(usize) -> Option<PathBuf>,
) -> SelectionResult {
    let mut new_selection = current_selection.clone();

    if let Some(anchor_idx) = anchor {
        let (start, end) = if anchor_idx < new_index {
            (anchor_idx, new_index)
        } else {
            (new_index, anchor_idx)
        };

        // Add range between anchor and focus
        for i in start..=end {
            if let Some(path) = get_item_path(i) {
                new_selection.insert(path);
            }
        }
    }

    if let Some(path) = get_item_path(new_index) {
        SelectionResult {
            selected_item: Some(new_index),
            selected_file: Some(path.clone()),
            selection_anchor: anchor,
            multi_selection: new_selection,
            selection_changed: true,
        }
    } else {
        SelectionResult {
            selected_item: Some(new_index),
            selected_file: None,
            selection_anchor: anchor,
            multi_selection: new_selection,
            selection_changed: true,
        }
    }
}

/// Handle keyboard navigation without modifiers
///
/// Clears selection and selects only the navigated item
pub fn handle_navigation_selection(new_index: usize, item_path: &Path) -> SelectionResult {
    let mut new_selection = FxHashSet::default();
    new_selection.insert(item_path.to_path_buf());

    SelectionResult {
        selected_item: Some(new_index),
        selected_file: Some(item_path.to_path_buf()),
        selection_anchor: Some(new_index),
        multi_selection: new_selection,
        selection_changed: true,
    }
}

/// Apply selection result to app state
///
/// This updates the app's selection state based on the result
pub fn apply_selection_result(app: &mut ImageViewerApp, result: SelectionResult) {
    app.selected_item = result.selected_item;
    app.selection_anchor = result.selection_anchor;
    app.multi_selection = result.multi_selection;

    // Note: selected_file and update_selected_thumbnail should be handled
    // by the caller after this function returns
}
