use eframe::egui;
use rust_i18n::t;

#[derive(Clone, Default)]
struct Navigation {
    open: bool,
    active: usize,
    widget_id: Option<egui::Id>,
}

pub(super) struct Output {
    pub preview: Option<String>,
    pub confirmed: Option<String>,
    pub cancelled: bool,
    pub open: bool,
    #[cfg(test)]
    response: egui::Response,
    #[cfg(test)]
    rows: Vec<egui::Rect>,
}

fn family(families: &[String], index: usize) -> &str {
    index
        .checked_sub(1)
        .map_or("", |index| families[index].as_str())
}

pub(super) fn render(ui: &mut egui::Ui, families: &[String], selected: &str) -> Output {
    let navigation_id = ui.make_persistent_id("ui_font_family_navigation");
    let mut nav = ui
        .ctx()
        .data(|data| data.get_temp::<Navigation>(navigation_id))
        .unwrap_or_default();
    let was_open = nav.open;
    if nav
        .widget_id
        .is_none_or(|id| !egui::ComboBox::is_open(ui.ctx(), id))
    {
        nav.open = false;
    }
    let current = if nav.open {
        family(families, nav.active)
    } else {
        selected
    };
    let label = if current.is_empty() {
        t!("settings.ui_font_default").to_string()
    } else {
        current.to_owned()
    };
    let mut confirmed = None;
    let mut preview = None;
    #[cfg(test)]
    let mut rows = Vec::new();
    let response = egui::ComboBox::from_id_salt("ui_font_family_selector")
        .selected_text(label)
        .height(260.0)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show_ui(ui, |ui| {
            let just_opened = !nav.open;
            if just_opened {
                nav.active = families
                    .iter()
                    .position(|name| name == selected)
                    .map_or(0, |index| index + 1);
                nav.open = true;
            }
            let before = nav.active;
            let (up, down, enter, escape, pointer_moved) = ui.input_mut(|input| {
                (
                    input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                    input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                    input.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                    input.key_pressed(egui::Key::Escape),
                    input.pointer.delta() != egui::Vec2::ZERO,
                )
            });
            if up {
                nav.active = nav.active.saturating_sub(1);
            }
            if down {
                nav.active = (nav.active + 1).min(families.len());
            }
            let keyboard_navigation = up || down;
            if enter && !escape {
                confirmed = Some(family(families, nav.active).to_owned());
                ui.close();
            }
            // Only actual pointer movement changes the hovered candidate. A font
            // can change row metrics under a stationary pointer; that must not
            // undo keyboard navigation or trigger a cascade of new previews.
            for index in 0..=families.len() {
                let name = family(families, index);
                let label = if index == 0 {
                    t!("settings.ui_font_default").to_string()
                } else {
                    name.to_owned()
                };
                let row = ui.selectable_label(nav.active == index, label);
                #[cfg(test)]
                rows.push(row.rect);
                if confirmed.is_none() && !escape {
                    if row.hovered() && pointer_moved && !keyboard_navigation {
                        nav.active = index;
                    }
                    if row.clicked() {
                        confirmed = Some(name.to_owned());
                        ui.close();
                    }
                }
                if index == nav.active && (just_opened || keyboard_navigation) {
                    row.scroll_to_me(Some(egui::Align::Center));
                }
            }
            if nav.active != before && confirmed.is_none() && !escape {
                preview = Some(family(families, nav.active).to_owned());
                ui.ctx().request_repaint();
            }
        })
        .response;
    let open = egui::ComboBox::is_open(ui.ctx(), response.id);
    let cancelled = (was_open || nav.open) && !open && confirmed.is_none();
    nav.open = open;
    nav.widget_id = Some(response.id);
    ui.ctx()
        .data_mut(|data| data.insert_temp(navigation_id, nav));
    Output {
        preview,
        confirmed,
        cancelled,
        open,
        #[cfg(test)]
        response,
        #[cfg(test)]
        rows,
    }
}

#[cfg(test)]
mod tests;
