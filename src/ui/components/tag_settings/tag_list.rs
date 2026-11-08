use super::{
    browser::navigate,
    model::{tag_labels, visible_tags, ListState},
};
use crate::{app::ImageViewerApp, domain::tag_tree};
use eframe::egui;
use rust_i18n::t;
use rustc_hash::FxHashSet;

pub(super) fn render(
    app: &mut ImageViewerApp,
    ui: &mut egui::Ui,
    list: &mut ListState,
    collapsed: &mut FxHashSet<i64>,
    height: f32,
) {
    let sorted = app.sorted_tag_ids();
    let ids = visible_tags(&app.tag_definitions, &sorted, &list.search, collapsed);
    if list.focus_selected && !list.selected.is_some_and(|id| ids.contains(&id)) {
        list.focus_selected = false;
    }
    let labels = tag_labels(&app.tag_definitions);
    let parents: FxHashSet<_> = app
        .tag_definitions
        .values()
        .filter_map(|tag| tag.parent_id)
        .collect();
    let searching = !list.search.trim().is_empty();
    let mut focused = false;
    let mut scroll = egui::ScrollArea::vertical()
        .id_salt("tag_browser_tag_list")
        .max_height(height)
        .min_scrolled_height(0.0)
        .auto_shrink([false, false]);
    if std::mem::take(&mut list.reset_scroll) {
        scroll = scroll.vertical_scroll_offset(0.0);
    }
    scroll.show(ui, |ui| {
        if ids.is_empty() {
            ui.label(t!("tags.browser_no_results"));
        }
        for id in &ids {
            let Some(tag) = app.tag_definitions.get(id) else {
                continue;
            };
            ui.push_id(("tag_browser_tag", id), |ui| {
                let output = super::tree_row::show(
                    ui,
                    super::tree_row::Row {
                        name: &tag.name,
                        color: tag.color.to_color32(),
                        depth: tag_tree::tag_depth(&app.tag_definitions, *id),
                        has_children: parents.contains(id),
                        expanded: !collapsed.contains(id) || searching,
                        selected: list.selected == Some(*id),
                        searching,
                    },
                );
                if output.toggle && !collapsed.remove(id) {
                    collapsed.insert(*id);
                }
                let response = output.response.on_hover_text(&labels[id]);
                if response.clicked() || response.gained_focus() {
                    list.select(*id);
                    app.tag_manager_delete_confirm = None;
                    response.request_focus();
                }
                if list.focus_selected && list.selected == Some(*id) {
                    response.request_focus();
                    response.scroll_to_me(Some(egui::Align::Center));
                    list.focus_selected = false;
                }
                focused |= response.has_focus();
                if response.has_focus() && !searching {
                    let left = ui.input_mut(|input| {
                        input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft)
                    });
                    let right = ui.input_mut(|input| {
                        input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight)
                    });
                    if left || right {
                        ui.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
                    }
                    if left && parents.contains(id) {
                        collapsed.insert(*id);
                    }
                    if right {
                        collapsed.remove(id);
                    }
                }
            });
        }
    });
    let old_selection = list.selected;
    navigate(ui, &ids, list, focused);
    if old_selection != list.selected {
        app.tag_manager_delete_confirm = None;
    }
}
