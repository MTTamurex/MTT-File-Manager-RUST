//! Context menu state management
//! Follows .cursorrules: single responsibility, < 300 lines

use std::path::PathBuf;

use eframe::egui;

/// A single item in the context menu (matches Files ContextMenuFlyoutItemViewModel)
#[derive(Clone)]
pub struct ContextMenuItem {
    /// Unique ID for shell commands (positive) or internal commands (negative)
    pub id: i32,
    /// Display text (mnemonics like & already removed)
    pub text: String,
    /// 16x16 icon texture
    pub icon: Option<egui::TextureHandle>,
    /// Submenu items (for expandable menus like 7-Zip, WinRAR)
    pub sub_items: Vec<ContextMenuItem>,
    /// True if this is a separator line
    pub is_separator: bool,
    /// Whether the item is clickable
    pub is_enabled: bool,
    /// Primary items appear in the header bar (Cut, Copy, Paste, Delete, Rename, Properties)
    pub is_primary: bool,
    /// Keyboard shortcut display (e.g., "Ctrl+C", "Alt+Ctrl+Enter")
    pub keyboard_shortcut: Option<String>,
    /// Shell command verb (e.g., "copy", "delete", "openas") for filtering
    pub command_string: Option<String>,
    /// Items that should go in "Show more options" overflow menu
    pub show_in_overflow: bool,
    /// True if this item has a pending submenu that needs on-demand loading
    pub has_pending_submenu: bool,
}

impl Default for ContextMenuItem {
    fn default() -> Self {
        Self {
            id: 0,
            text: String::new(),
            icon: None,
            sub_items: Vec::new(),
            is_separator: false,
            is_enabled: true,
            is_primary: false,
            keyboard_shortcut: None,
            command_string: None,
            show_in_overflow: false,
            has_pending_submenu: false,
        }
    }
}

impl ContextMenuItem {
    /// Creates a new regular menu item
    pub fn new(id: i32, text: impl Into<String>) -> Self {
        Self {
            id,
            text: text.into(),
            is_enabled: true,
            ..Default::default()
        }
    }
    
    /// Creates a separator
    pub fn separator() -> Self {
        Self {
            is_separator: true,
            ..Default::default()
        }
    }
    
    /// Creates a primary item (appears in header bar)
    pub fn primary(id: i32, text: impl Into<String>) -> Self {
        Self {
            id,
            text: text.into(),
            is_primary: true,
            is_enabled: true,
            ..Default::default()
        }
    }
    
    /// Builder: add keyboard shortcut
    pub fn with_shortcut(mut self, shortcut: impl Into<String>) -> Self {
        self.keyboard_shortcut = Some(shortcut.into());
        self
    }
    
    /// Builder: add icon
    pub fn with_icon(mut self, icon: egui::TextureHandle) -> Self {
        self.icon = Some(icon);
        self
    }
    
    /// Builder: set command string (shell verb)
    pub fn with_command(mut self, cmd: impl Into<String>) -> Self {
        self.command_string = Some(cmd.into());
        self
    }
    
    /// Builder: mark as overflow item
    pub fn in_overflow(mut self) -> Self {
        self.show_in_overflow = true;
        self
    }
    
    /// Builder: set enabled state
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.is_enabled = enabled;
        self
    }
    
    /// Builder: add sub-items
    pub fn with_subitems(mut self, items: Vec<ContextMenuItem>) -> Self {
        self.sub_items = items;
        self
    }
}

impl std::fmt::Debug for ContextMenuItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContextMenuItem")
            .field("id", &self.id)
            .field("text", &self.text)
            .field("sub_items_count", &self.sub_items.len())
            .field("is_separator", &self.is_separator)
            .field("is_enabled", &self.is_enabled)
            .field("is_primary", &self.is_primary)
            .finish()
    }
}

/// Context menu state
#[derive(Clone)]
pub struct ContextMenuState {
    pub is_open: bool,
    pub position: egui::Pos2,
    pub item_index: Option<usize>,
    pub target_path: Option<PathBuf>,
    pub is_empty_area: bool,
    
    /// Dynamic items extracted from Shell or built for empty area
    pub items: Vec<ContextMenuItem>,
    
    /// The ID of the command selected by the user (positive for Shell, negative for internal)
    pub selected_command_id: Option<i32>,
    
    /// Native shell context (holds IContextMenu and other COM objects alive)
    /// Stored as Any to keep the application layer agnostic of Win32 types
    /// Note: Not thread-safe - must only be accessed from main thread
    pub native_context: Option<std::rc::Rc<dyn std::any::Any>>,
}

impl Default for ContextMenuState {
    fn default() -> Self {
        Self {
            is_open: false,
            position: egui::Pos2::ZERO,
            item_index: None,
            target_path: None,
            is_empty_area: false,
            items: Vec::new(),
            selected_command_id: None,
            native_context: None,
        }
    }
}

impl ContextMenuState {
    /// Creates a new context menu state
    pub fn new() -> Self {
        Self::default()
    }

    /// Opens the context menu at the specified position
    pub fn open(
        &mut self,
        position: egui::Pos2,
        item_index: Option<usize>,
        target_path: Option<PathBuf>,
        is_empty_area: bool,
    ) {
        self.is_open = true;
        self.position = position;
        self.item_index = item_index;
        self.target_path = target_path;
        self.is_empty_area = is_empty_area;
    }

    /// Closes the context menu
    pub fn close(&mut self) {
        self.is_open = false;
        self.item_index = None;
        self.target_path = None;
        self.is_empty_area = false;
        self.items.clear();
        self.selected_command_id = None;
        self.native_context = None;
    }

    /// Checks if the context menu is open for a specific item
    pub fn is_open_for_item(&self, index: usize) -> bool {
        self.is_open && self.item_index == Some(index)
    }

    /// Checks if the context menu is open for empty area
    pub fn is_open_for_empty_area(&self) -> bool {
        self.is_open && self.is_empty_area
    }
}

impl std::fmt::Debug for ContextMenuState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContextMenuState")
            .field("is_open", &self.is_open)
            .field("position", &self.position)
            .field("item_index", &self.item_index)
            .field("target_path", &self.target_path)
            .field("is_empty_area", &self.is_empty_area)
            .field("items_count", &self.items.len())
            .field("selected_command_id", &self.selected_command_id)
            .finish()
    }
}
