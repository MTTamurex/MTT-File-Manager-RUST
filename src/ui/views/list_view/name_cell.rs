//! Name-column painting and inline rename, at the column's actual position.
use super::{truncate_text_for_column, ColumnWidths, ListViewContext, ListViewOperations};
use crate::domain::file_entry::FileEntry;
use eframe::egui::{self, Color32, FontId, Pos2, Rect, Ui};

pub(super) fn first_tag_color(item: &FileEntry, ctx: &ListViewContext) -> Option<Color32> {
    crate::domain::file_tag::tag_ids_for_path(ctx.tag_assignments, &item.path)
        .and_then(|ids| ids.iter().find_map(|id| ctx.tag_definitions.get(id)))
        .map(|tag| tag.color.to_color32())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render(
    ui: &mut Ui,
    index: usize,
    item: &FileEntry,
    row: Rect,
    widths: &ColumnWidths,
    ctx: &mut ListViewContext,
    ops: &mut dyn ListViewOperations,
    response: &egui::Response,
    text_color: Color32,
    opacity: f32,
) {
    let rect = Rect::from_min_size(
        row.min + egui::vec2(widths.name_offset, 0.0),
        egui::vec2(widths.name, row.height()),
    );
    let clip = rect.intersect(ui.clip_rect());
    let old_clip = ui.clip_rect();
    ui.set_clip_rect(clip);
    let tint = Color32::WHITE.gamma_multiply(
        opacity * crate::ui::clipboard_feedback::icon_opacity(ui.ctx(), &item.path),
    );
    super::item_renderer_details::render_item_icon(ui, item, ctx, ops, rect, tint);
    let tag_color = first_tag_color(item, ctx);
    let tag_offset = if tag_color.is_some() { 12.0 } else { 0.0 };
    if let Some(color) = tag_color {
        crate::ui::tag_icon::paint_filled(
            ui.painter(),
            Pos2::new(rect.min.x + 27.0, rect.center().y),
            9.0,
            color.gamma_multiply(opacity),
        );
    }
    let name_rect = Rect::from_min_size(
        rect.min + egui::vec2(24.0 + tag_offset, 2.0),
        egui::vec2(
            (widths.name - 30.0 - tag_offset).max(0.0),
            row.height() - 4.0,
        ),
    );
    if !ctx.compact
        && !ctx.is_recycle_bin_view
        && !ctx.global_search_active
        && ctx.renaming_state.is_none()
        && (ctx.selected_item == Some(index) || response.clicked() || response.double_clicked())
        && crate::ui::components::inline_rename::selected_name_click_starts_rename(
            ui,
            ctx.rename_click_scope,
            response,
            &item.path,
            name_rect,
            ctx.selected_item == Some(index) && ctx.multi_selection.len() <= 1,
        )
    {
        ctx.rename_requested_item = Some(index);
    }
    let renaming = ctx
        .renaming_state
        .as_ref()
        .is_some_and(|(path, _)| *path == item.path);
    if renaming {
        if let Some((_, rename_text)) = &ctx.renaming_state {
            let mut text = rename_text.clone();
            ui.scope_builder(egui::UiBuilder::new().max_rect(name_rect), |ui| {
                let response = ui.add(
                    egui::TextEdit::singleline(&mut text)
                        .horizontal_align(egui::Align::Min)
                        .id_source("rename_input_list"),
                );
                let menu = crate::ui::components::inline_rename::add_text_context_menu(
                    ui,
                    &response,
                    &mut text,
                    ctx.focus_rename,
                );
                if ctx.focus_rename {
                    response.request_focus();
                    if let Some(mut state) =
                        egui::widgets::text_edit::TextEditState::load(ui.ctx(), response.id)
                    {
                        let end = if item.is_dir {
                            text.chars().count()
                        } else {
                            text.rfind('.')
                                .map(|pos| text[..pos].chars().count())
                                .filter(|&pos| pos > 0)
                                .unwrap_or(text.chars().count())
                        };
                        state
                            .cursor
                            .set_char_range(Some(egui::text::CCursorRange::two(
                                egui::text::CCursor::new(0),
                                egui::text::CCursor::new(end),
                            )));
                        state.store(ui.ctx(), response.id);
                    }
                }
                if (response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)))
                    || (response.lost_focus() && !menu.menu_active)
                {
                    ops.rename_with_shell(index);
                }
            });
            if let Some((_, rename_text)) = ctx.renaming_state.as_mut() {
                *rename_text = text;
            }
        }
    } else {
        let font = FontId::proportional(12.0);
        let name = crate::ui::components::item_slot::display_name_for_item(item);
        let display = truncate_text_for_column(&name, name_rect.width(), &font, ui);
        ui.painter().text(
            rect.min + egui::vec2(24.0 + tag_offset, 5.0),
            egui::Align2::LEFT_TOP,
            display,
            font,
            text_color,
        );
    }
    ui.set_clip_rect(old_clip);
}
