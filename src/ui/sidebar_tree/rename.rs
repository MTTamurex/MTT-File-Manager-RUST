//! Inline folder-name editing for sidebar tree nodes.

use super::SidebarTreeAction;
use crate::application::sidebar_tree::SidebarFolderRename;
use eframe::egui;

fn rendered_id() -> egui::Id {
    egui::Id::new("sidebar_folder_rename_rendered")
}

pub(crate) fn begin_frame(ctx: &egui::Context) {
    ctx.data_mut(|data| data.remove::<egui::Id>(rendered_id()));
}

pub(crate) fn cancel(ctx: &egui::Context, editing: &mut Option<SidebarFolderRename>) {
    if let Some(editing) = editing.take() {
        ctx.memory_mut(|memory| memory.surrender_focus(editing.target.editor_id()));
    }
}

pub(crate) fn finish_frame(ctx: &egui::Context, editing: &mut Option<SidebarFolderRename>) {
    let rendered = ctx.data(|data| data.get_temp::<egui::Id>(rendered_id()));
    if editing
        .as_ref()
        .is_some_and(|editing| Some(editing.target.node_id) != rendered)
    {
        cancel(ctx, editing);
    } else if let Some(editing) = editing {
        editing.request_focus = false;
    }
}

pub(crate) fn render(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    editing: &SidebarFolderRename,
) -> Option<SidebarTreeAction> {
    if !ui.is_rect_visible(rect) {
        return None;
    }
    ui.data_mut(|data| data.insert_temp(rendered_id(), editing.target.node_id));
    let id = editing.target.editor_id();
    let focus = editing.request_focus;
    let mut text = editing.text.clone();
    let response = ui.put(
        rect,
        egui::TextEdit::singleline(&mut text)
            .id(id)
            .font(egui::FontId::proportional(11.0))
            .desired_width(rect.width())
            .margin(egui::Margin::symmetric(3, 1)),
    );
    if focus {
        response.request_focus();
        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(text.chars().count()),
                )));
            state.store(ui.ctx(), id);
        }
    }
    let text_menu = crate::ui::components::inline_rename::add_text_context_menu(
        ui, &response, &mut text, focus,
    );
    let escape = ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    let enter = ui.input(|input| input.key_pressed(egui::Key::Enter));
    if escape || (!focus && response.lost_focus() && !enter && !text_menu.menu_active) {
        Some(SidebarTreeAction::CancelRename)
    } else if !focus && enter && (response.has_focus() || response.lost_focus()) {
        Some(SidebarTreeAction::CommitRename {
            node_id: editing.target.node_id,
            new_name: text,
        })
    } else if response.changed() || text_menu.text_changed {
        Some(SidebarTreeAction::RenameChanged(text))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(
        ctx: &egui::Context,
        name: &str,
        focus: bool,
        events: Vec<egui::Event>,
    ) -> Option<SidebarTreeAction> {
        let mut action = None;
        let mut editing =
            SidebarFolderRename::new(crate::application::sidebar_tree::SidebarFolderTarget::new(
                egui::Id::new("tree"),
                std::path::Path::new(r"C:\Folder\Subfolder"),
                name,
            ));
        editing.request_focus = focus;
        let _ = ctx.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| {
                action = render(
                    ui,
                    egui::Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(240.0, 24.0)),
                    &editing,
                );
            },
        );
        action
    }

    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn sidebar_tree_rename_edits_and_commits_the_original_node_path() {
        let ctx = egui::Context::default();
        frame(&ctx, "Folder α", true, vec![]);
        let action = frame(
            &ctx,
            "Folder α",
            false,
            vec![egui::Event::Text("Renamed".into())],
        );
        assert!(
            matches!(action, Some(SidebarTreeAction::RenameChanged(ref name)) if name == "Renamed")
        );
        let action = frame(&ctx, "Renamed", false, vec![key(egui::Key::Enter)]);
        assert!(
            matches!(action, Some(SidebarTreeAction::CommitRename { node_id, new_name })
            if node_id == egui::Id::new("tree").with(std::path::Path::new(r"C:\Folder\Subfolder")) && new_name == "Renamed")
        );
    }

    #[test]
    fn sidebar_tree_rename_escape_cancels_without_committing() {
        let ctx = egui::Context::default();
        frame(&ctx, "Folder", true, vec![]);
        let action = frame(&ctx, "Folder", false, vec![key(egui::Key::Escape)]);
        assert!(matches!(action, Some(SidebarTreeAction::CancelRename)));
    }
}
