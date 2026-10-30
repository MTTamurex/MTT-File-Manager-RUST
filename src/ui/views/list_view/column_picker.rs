use super::{column_choices, columns, ListViewContext};
use crate::domain::list_columns::ColumnId;
use eframe::egui::{self, Ui};
use rust_i18n::t;

fn checkbox(ui: &mut Ui, ctx: &mut ListViewContext, id: ColumnId, label: &str) {
    let scope = columns::scope(ctx);
    let mut visible = ctx.columns.preferences.layout(scope).is_visible(id);
    let response = ui.add_enabled(
        id != ColumnId::Name,
        egui::Checkbox::new(&mut visible, label),
    );
    if response.changed() {
        ctx.columns
            .preferences
            .layout_mut(scope)
            .set_visible(id, visible);
    }
}

pub(super) fn menu(ui: &mut Ui, ctx: &mut ListViewContext, owner: egui::Id) {
    let scope = columns::scope(ctx);
    let ids: Vec<_> = ctx
        .columns
        .preferences
        .layout(scope)
        .columns
        .iter()
        .map(|column| column.id)
        .collect();
    egui::ScrollArea::vertical()
        .max_height(300.0)
        .show(ui, |ui| {
            for id in ids {
                if id == ColumnId::Status && !ctx.is_onedrive_folder {
                    continue;
                }
                checkbox(ui, ctx, id, &columns::label(ctx, id));
            }
        });
    ui.separator();
    if ui.button(t!("column_headers.more_columns")).clicked() {
        ctx.columns.show_picker = true;
        ctx.columns.picker_owner = Some(owner);
        ui.ctx().data_mut(|data| {
            data.insert_temp(owner.with("column_picker").with("center_on_open"), true)
        });
        ui.close();
    }
}

pub(super) fn dialog(ui: &mut Ui, ctx: &mut ListViewContext) {
    if !ctx.columns.show_picker || ctx.columns.picker_owner != Some(ctx.rename_click_scope) {
        return;
    }
    let mut open = true;
    picker_window(ui.ctx(), ctx.rename_click_scope.with("column_picker"))
        .open(&mut open)
        .show(ui.ctx(), |ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            ui.add(
                egui::TextEdit::singleline(&mut ctx.columns.picker_search)
                    .desired_width(f32::INFINITY)
                    .hint_text(t!("column_headers.search")),
            );
            if !ctx.columns.catalog_loaded && !ctx.columns.catalog_failed {
                ui.spinner();
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(100));
            }
            if ctx.columns.catalog_failed {
                ui.label(t!("column_headers.catalog_failed"));
            }
            let search = ctx.columns.picker_search.to_lowercase();
            let choices = column_choices::choices(
                ctx.columns.preferences.layout(columns::scope(ctx)),
                &ctx.columns.catalog,
                columns::scope(ctx),
                |id| columns::label(ctx, id),
            );
            choices_scroll(ui, |ui| {
                for (id, label, canonical) in choices {
                    if id == ColumnId::Status && !ctx.is_onedrive_folder {
                        continue;
                    }
                    if !search.is_empty()
                        && !label.to_lowercase().contains(&search)
                        && !canonical.to_lowercase().contains(&search)
                    {
                        continue;
                    }
                    ui.push_id(id, |ui| checkbox(ui, ctx, id, &label));
                }
            });
        });
    ctx.columns.show_picker = open;
}

fn picker_window(ctx: &egui::Context, id: egui::Id) -> egui::Window<'static> {
    let center = ctx.viewport_rect().center();
    let window = egui::Window::new(t!("column_headers.choose_columns").to_string())
        .id(id)
        .collapsible(false)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(center)
        .default_width(340.0)
        .min_width(280.0)
        .max_width(440.0)
        .resizable(true);
    // Center on each opening, including when an older position was remembered.
    // Do not anchor: the user must still be able to move the open window.
    if ctx
        .data_mut(|data| data.remove_temp::<bool>(id.with("center_on_open")))
        .unwrap_or(false)
    {
        // Disable dragging only on the opening frame: egui's title-bar drag
        // otherwise restores the previous position over current_pos.
        window.movable(false).current_pos(center)
    } else {
        window
    }
}

fn choices_scroll(
    ui: &mut Ui,
    contents: impl FnOnce(&mut Ui),
) -> egui::scroll_area::ScrollAreaOutput<()> {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .max_height(440.0)
        .show(ui, contents)
}

#[cfg(test)]
#[path = "column_picker_tests.rs"]
mod tests;
