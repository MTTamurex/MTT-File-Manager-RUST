use eframe::egui;
use rust_i18n::t;

mod popup;
mod typeahead;

#[derive(Clone, Default)]
struct Navigation {
    open: bool,
    active: usize,
    widget_id: Option<egui::Id>,
    follow_active: bool,
    typeahead: typeahead::TypeAhead,
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
    #[cfg(test)]
    viewport: Option<egui::Rect>,
    #[cfg(test)]
    active: usize,
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
    if nav.widget_id.is_none_or(|id| !popup::is_open(ui.ctx(), id)) {
        nav.open = false;
    }
    let current = if nav.open {
        family(families, nav.active)
    } else {
        selected
    };
    let default_label = t!("settings.ui_font_default").to_string();
    let label = if current.is_empty() {
        default_label.clone()
    } else {
        current.to_owned()
    };
    let mut confirmed = None;
    let mut preview = None;
    #[cfg(test)]
    let mut rows = Vec::new();
    #[cfg(test)]
    let mut viewport = None;
    let response = popup::show(ui, &label, |ui| {
        #[cfg(test)]
        {
            viewport = Some(ui.clip_rect());
        }
        let just_opened = !nav.open;
        if just_opened {
            nav.active = families
                .iter()
                .position(|name| name == selected)
                .map_or(0, |index| index + 1);
            nav.open = true;
            nav.follow_active = true;
            nav.typeahead.clear();
        }
        let before = nav.active;
        let (up, down, enter, escape, pointer_moved, manual_scroll, edits, now) =
            ui.input_mut(|input| {
                let mut edits = Vec::new();
                let manual_scroll = input
                    .events
                    .iter()
                    .any(|event| matches!(event, egui::Event::MouseWheel { .. }))
                    || (input.pointer.any_down() && input.pointer.delta() != egui::Vec2::ZERO);
                // Spaces belong to names such as "Test Font", not the focused
                // selectable row's default Space-to-click keyboard shortcut.
                input.events.retain(|event| match event {
                    egui::Event::Text(value) => {
                        edits.push(typeahead::Edit::Text(value.clone()));
                        false
                    }
                    egui::Event::Key {
                        key: egui::Key::Backspace,
                        pressed,
                        modifiers,
                        ..
                    } if !modifiers.ctrl && !modifiers.command && !modifiers.alt => {
                        if *pressed {
                            edits.push(typeahead::Edit::Backspace);
                        }
                        false
                    }
                    egui::Event::Key {
                        key: egui::Key::Space,
                        ..
                    } => false,
                    _ => true,
                });
                (
                    input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                    input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                    input.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                    input.key_pressed(egui::Key::Escape),
                    input.pointer.delta() != egui::Vec2::ZERO,
                    manual_scroll,
                    edits,
                    input.time,
                )
            });
        if up {
            nav.active = nav.active.saturating_sub(1);
        }
        if down {
            nav.active = (nav.active + 1).min(families.len());
        }
        if up || down || manual_scroll {
            nav.typeahead.clear();
        }
        let text_navigation = !edits.is_empty();
        let mut prefix = None;
        for edit in edits {
            prefix = match edit {
                typeahead::Edit::Text(text) => nav.typeahead.input(&text, false, now),
                typeahead::Edit::Backspace => nav.typeahead.input("", true, now),
            };
        }
        if let Some(prefix) = prefix {
            if let Some(index) = typeahead::find(families, &default_label, &prefix) {
                nav.active = index;
            }
        }
        let keyboard_navigation = up || down || text_navigation;
        if manual_scroll {
            nav.follow_active = false;
        }
        if keyboard_navigation {
            nav.follow_active = true;
            // The popup owns arrow navigation. Do not let egui move focus
            // independently to a different row while we update the highlight.
            ui.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
        }
        if enter && !escape {
            confirmed = Some(family(families, nav.active).to_owned());
            ui.close();
        }
        // Only actual pointer movement changes the hovered candidate. A font
        // can change row metrics under a stationary pointer; that must not
        // undo keyboard navigation or trigger a cascade of new previews.
        let mut active_row = None;
        for index in 0..=families.len() {
            let name = family(families, index);
            let label = if index == 0 {
                default_label.clone()
            } else {
                name.to_owned()
            };
            let row = ui.selectable_label(nav.active == index, label);
            #[cfg(test)]
            rows.push(row.rect);
            if confirmed.is_none() && !escape {
                if row.hovered() && pointer_moved && !keyboard_navigation {
                    nav.active = index;
                    nav.follow_active = !manual_scroll;
                    nav.typeahead.clear();
                }
                if row.clicked() {
                    confirmed = Some(name.to_owned());
                    ui.close();
                }
            }
            if index == nav.active {
                active_row = Some(row);
            }
        }
        if let Some(row) = active_row {
            if just_opened || keyboard_navigation {
                row.request_focus();
            }
            let clip = ui.clip_rect();
            let outside = row.rect.top() < clip.top() || row.rect.bottom() > clip.bottom();
            if nav.follow_active && (just_opened || outside) {
                // Recheck after font/size changes too, not only on keypress.
                // Animated centering otherwise lags behind held arrow keys.
                row.scroll_to_me_animation(
                    Some(egui::Align::Center),
                    egui::style::ScrollAnimation::none(),
                );
            }
        }
        if nav.active != before && confirmed.is_none() && !escape {
            preview = Some(family(families, nav.active).to_owned());
            ui.ctx().request_repaint();
        }
    })
    .response
    .on_hover_text(t!("settings.ui_font_typeahead_hint"));
    let open = popup::is_open(ui.ctx(), response.id);
    let cancelled = (was_open || nav.open) && !open && confirmed.is_none();
    nav.open = open;
    nav.widget_id = Some(response.id);
    #[cfg(test)]
    let active = nav.active;
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
        #[cfg(test)]
        viewport,
        #[cfg(test)]
        active,
    }
}

#[cfg(test)]
mod scroll_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod typeahead_tests;
