use super::{
    browser::navigate,
    model::{tag_labels, visible_rules, ListState},
};
use crate::{app::ImageViewerApp, ui::components::auto_tag_settings};
use eframe::egui;
use rust_i18n::t;
use rustc_hash::FxHashSet;

pub(super) fn render(
    app: &mut ImageViewerApp,
    ui: &mut egui::Ui,
    list: &mut ListState,
    height: f32,
) {
    let labels = tag_labels(&app.tag_definitions);
    let ids = visible_rules(&app.auto_tag_rules, &labels, &list.search);
    let visible: FxHashSet<_> = ids.iter().copied().collect();
    if list.focus_selected && !list.selected.is_some_and(|id| visible.contains(&id)) {
        list.focus_selected = false;
    }
    let mut focused = false;
    let old_selection = list.selected;
    let mut scroll = egui::ScrollArea::vertical()
        .id_salt("tag_browser_rule_list")
        .max_height(height)
        .min_scrolled_height(0.0)
        .auto_shrink([false, false]);
    if std::mem::take(&mut list.reset_scroll) {
        scroll = scroll.vertical_scroll_offset(0.0);
    }
    let mut updates = Vec::new();
    scroll.show(ui, |ui| {
        if ids.is_empty() {
            ui.label(t!("tags.browser_no_results"));
        }
        for rule in app
            .auto_tag_rules
            .iter()
            .filter(|rule| visible.contains(&rule.id))
        {
            ui.push_id(("tag_browser_rule", rule.id), |ui| {
                ui.horizontal(|ui| {
                    let mut enabled = rule.enabled;
                    if ui
                        .checkbox(&mut enabled, "")
                        .on_hover_text(t!("tags.auto_enabled_tooltip"))
                        .changed()
                    {
                        updates.push((rule.id, enabled));
                    }
                    let path = rule.folder_path.to_string_lossy();
                    let tag = labels.get(&rule.tag_id).map(String::as_str).unwrap_or("");
                    let status = auto_tag_settings::rule_status(app, rule);
                    let response = super::rule_row::show(
                        ui,
                        &path,
                        tag,
                        &status,
                        list.selected == Some(rule.id),
                    )
                    .on_hover_text(format!("{path}\n{tag}\n{status}"));
                    if response.clicked() || response.gained_focus() {
                        list.select(rule.id);
                        response.request_focus();
                    }
                    if list.focus_selected && list.selected == Some(rule.id) {
                        response.request_focus();
                        response.scroll_to_me(Some(egui::Align::Center));
                        list.focus_selected = false;
                    }
                    focused |= response.has_focus();
                });
            });
            ui.add_space(4.0);
        }
    });
    for (id, enabled) in updates {
        if app.app_state_db.set_auto_tag_rule_enabled(id, enabled) {
            app.reload_auto_tag_rules();
        }
    }
    navigate(ui, &ids, list, focused);
    if old_selection != list.selected {
        app.auto_tag_delete_confirm = None;
    }
}
