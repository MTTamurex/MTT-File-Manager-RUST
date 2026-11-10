//! Transient command discovery; execution reuses the application's existing operations.
pub(crate) mod availability;
pub(crate) mod catalog;
mod execution;
pub(crate) mod input_hints;
pub(crate) mod search;

use crate::app::ImageViewerApp;
use eframe::egui;

#[derive(Default)]
pub(crate) struct CommandPaletteState {
    pub active: bool,
    pub query: String,
    pub selected: usize,
    pub request_focus: bool,
    pub reveal_selected: bool,
    pub pointer_position: Option<egui::Pos2>,
    previous_focus: Option<egui::Id>,
}

impl CommandPaletteState {
    pub fn open(&mut self, previous_focus: Option<egui::Id>) {
        *self = Self {
            active: true,
            request_focus: true,
            previous_focus,
            ..Self::default()
        };
    }

    pub fn close(&mut self, ctx: &egui::Context) {
        if let Some(id) = self.previous_focus.take() {
            ctx.memory_mut(|memory| memory.request_focus(id));
        } else {
            ctx.memory_mut(|memory| memory.surrender_focus(crate::ui::command_palette::input_id()));
        }
        self.active = false;
        self.query.clear();
        self.selected = 0;
        ctx.request_repaint();
    }

    pub fn move_selection(&mut self, count: usize, backwards: bool) {
        if count == 0 {
            self.selected = 0;
        } else if backwards {
            self.selected = (self.selected.min(count - 1) + count - 1) % count;
        } else {
            self.selected = (self.selected.min(count - 1) + 1) % count;
        }
        self.reveal_selected = true;
    }
}

impl ImageViewerApp {
    pub(crate) fn can_open_command_palette(&self, ctx: &egui::Context) -> bool {
        !self.command_palette_blocked()
            && !ctx.egui_wants_keyboard_input()
            && !egui::Popup::is_any_open(ctx)
            && ctx.memory(|memory| memory.top_modal_layer().is_none())
            && !crate::app::operations::tabs::transfer_pending(ctx)
    }

    pub(crate) fn command_palette_blocked(&self) -> bool {
        self.tab_navigation_blocked()
            || self.shortcut_editor.is_capturing()
            || self.is_address_editing
            || self.show_address_history_menu
            || self.pending_tag_cascade.is_some()
            || self.pending_shell_open_confirmation.is_some()
            || self.external_drop_active
            || self.update_dialog_blocks_command_palette()
    }

    fn update_dialog_blocks_command_palette(&self) -> bool {
        #[cfg(feature = "online-updates")]
        {
            self.updates.show_dialog
        }
        #[cfg(not(feature = "online-updates"))]
        {
            false
        }
    }
}

#[cfg(test)]
mod tests;
