use super::{
    model::{BrowserState, Tab},
    rule_list, tag_list,
};
use crate::{
    app::ImageViewerApp,
    ui::components::{auto_tag_settings, tag_manager_modal},
};
use eframe::egui;
use rust_i18n::t;

pub(super) fn navigate(
    ui: &mut egui::Ui,
    ids: &[i64],
    list: &mut super::model::ListState,
    focused: bool,
) {
    if !focused {
        return;
    }
    let direction = ui.input_mut(|input| {
        if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
            Some(true)
        } else if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
            Some(false)
        } else {
            None
        }
    });
    if let Some(forward) = direction {
        ui.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
        if let Some(id) = super::model::adjacent(ids, list.selected, forward) {
            list.select(id);
            list.focus_selected = true;
            ui.ctx().request_repaint();
        }
    }
}

pub(crate) fn render_browser(app: &mut ImageViewerApp, ui: &mut egui::Ui) {
    ui.scope(|ui| {
        let scroll = &mut ui.spacing_mut().scroll;
        scroll.floating_allocated_width = scroll.bar_width + 8.0;
        render_browser_content(app, ui);
    });
}

fn render_browser_content(app: &mut ImageViewerApp, ui: &mut egui::Ui) {
    // Scope to the host window: modal and settings never share widget IDs,
    // selections or scroll positions, but switching tabs preserves each one.
    let id = ui.id().with("tag_browser");
    let mut state = ui
        .ctx()
        .data_mut(|data| data.get_temp::<BrowserState>(id))
        .unwrap_or_else(|| BrowserState {
            // Start with categories collapsed rather than restoring the long
            // expanded list that this browser replaces.
            collapsed: app
                .tag_definitions
                .values()
                .filter_map(|tag| tag.parent_id)
                .collect(),
            ..Default::default()
        });
    state
        .tags
        .reconcile(|id| app.tag_definitions.contains_key(&id));
    state
        .rules
        .reconcile(|id| app.auto_tag_rules.iter().any(|rule| rule.id == id));
    state
        .collapsed
        .retain(|id| app.tag_definitions.contains_key(id));
    let old_tab = state.tab;
    let actions = super::toolbar::show(ui, state.tab);
    if let Some(tab) = actions.tab {
        state.tab = tab;
    }
    if actions.add {
        state.creation = Some(state.tab);
        app.tag_manager_delete_confirm = None;
        app.auto_tag_delete_confirm = None;
    }
    if old_tab != state.tab {
        state.tags.focus_selected = false;
        state.rules.focus_selected = false;
        app.tag_manager_delete_confirm = None;
        app.auto_tag_delete_confirm = None;
    }
    ui.add_space(12.0);
    let list = match state.tab {
        Tab::Tags => &mut state.tags,
        Tab::AutoTag => &mut state.rules,
    };
    let width = ui.available_width();
    if ui
        .add_sized(
            [width, ui.spacing().interact_size.y],
            egui::TextEdit::singleline(&mut list.search)
                .hint_text(t!("tags.browser_search"))
                .desired_width(width),
        )
        .changed()
    {
        list.search_changed();
    }
    ui.add_space(8.0);
    super::layout::show(ui, |ui, pane, height| match pane {
        super::layout::Pane::List => render_list(app, ui, &mut state, height),
        super::layout::Pane::Details => render_details(app, ui, &mut state, height),
    });
    super::creation::show(app, ui.ctx(), id, &mut state.creation);
    ui.ctx().data_mut(|data| data.insert_temp(id, state));
}

#[cfg(test)]
mod tests;

fn render_list(app: &mut ImageViewerApp, ui: &mut egui::Ui, state: &mut BrowserState, height: f32) {
    match state.tab {
        Tab::Tags => tag_list::render(app, ui, &mut state.tags, &mut state.collapsed, height),
        Tab::AutoTag => rule_list::render(app, ui, &mut state.rules, height),
    }
}

fn render_details(
    app: &mut ImageViewerApp,
    ui: &mut egui::Ui,
    state: &mut BrowserState,
    height: f32,
) {
    let list = match state.tab {
        Tab::Tags => &mut state.tags,
        Tab::AutoTag => &mut state.rules,
    };
    let scroll_id = (
        "tag_browser_details",
        state.tab == Tab::AutoTag,
        list.selected,
    );
    egui::ScrollArea::vertical()
        .id_salt(scroll_id)
        .max_height(height)
        .min_scrolled_height(0.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if let Some(id) = list.selected {
                match state.tab {
                    Tab::Tags => tag_manager_modal::render_tag_details(app, ui, id),
                    Tab::AutoTag => auto_tag_settings::render_rule_details(app, ui, id),
                }
            } else {
                ui.add_space(24.0);
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(t!("tags.browser_select"))
                            .color(crate::ui::theme::secondary_text_color(ui.ctx())),
                    )
                    .wrap(),
                );
            }
        });
}
