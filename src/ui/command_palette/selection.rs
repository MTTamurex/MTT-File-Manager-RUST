use crate::app::command_palette::CommandPaletteState;
use eframe::egui;

pub(super) fn pointer_moved(state: &mut CommandPaletteState, ctx: &egui::Context) -> bool {
    let position = ctx.input(|input| input.pointer.hover_pos());
    let moved = position.is_some() && position != state.pointer_position;
    state.pointer_position = position;
    moved
}

pub(super) fn select_hovered(state: &mut CommandPaletteState, index: usize, ctx: &egui::Context) {
    if state.selected != index {
        state.selected = index;
        state.reveal_selected = false;
        // Earlier rows may already have been painted. Repaint this same frame
        // so there is never an old keyboard highlight alongside the hovered row.
        ctx.request_discard("command palette pointer selection changed");
    }
}
