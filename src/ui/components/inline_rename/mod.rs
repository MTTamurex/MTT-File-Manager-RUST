mod context_menu;

pub(crate) use context_menu::add_text_context_menu;

#[derive(Clone, Copy, Default)]
pub(crate) struct TextContextMenuResult {
    pub menu_active: bool,
    pub text_changed: bool,
}

use eframe::egui::{self, Id, Rect, Response, Ui};
use std::path::Path;

#[derive(Clone, Debug, PartialEq)]
struct PendingNameClick {
    path: std::path::PathBuf,
    rect: Rect,
    deadline: f64,
}

fn pending_name_click_id(scope: Id) -> Id {
    scope.with("pending_inline_rename_name_click")
}

#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct NameClickFlags {
    is_selected: bool,
    clicked_on_name: bool,
    double_clicked: bool,
}

fn update_pending_name_click(
    pending: Option<PendingNameClick>,
    item_path: &Path,
    name_rect: Rect,
    flags: NameClickFlags,
    now: f64,
    delay: f64,
) -> (Option<PendingNameClick>, bool) {
    if flags.double_clicked {
        return (None, false);
    }

    if pending
        .as_ref()
        .is_some_and(|pending| pending.path == item_path && now >= pending.deadline)
    {
        return (None, true);
    }

    if flags.clicked_on_name && flags.is_selected {
        return (
            Some(PendingNameClick {
                path: item_path.to_path_buf(),
                rect: name_rect,
                deadline: now + delay,
            }),
            false,
        );
    }

    (pending, false)
}

/// Clears a delayed rename gesture when the user clicks outside its name field.
pub(crate) fn clear_pending_name_click_outside(
    ctx: &egui::Context,
    scope: Id,
    selected_path: Option<&Path>,
) {
    let (click_pos, secondary_clicked, keyboard_or_scroll) = ctx.input(|input| {
        let primary_clicked = input.pointer.button_clicked(egui::PointerButton::Primary);
        (
            primary_clicked
                .then(|| input.pointer.interact_pos())
                .flatten(),
            input.pointer.button_clicked(egui::PointerButton::Secondary),
            input.events.iter().any(|event| {
                matches!(
                    event,
                    egui::Event::Key { pressed: true, .. }
                        | egui::Event::Text(_)
                        | egui::Event::Paste(_)
                        | egui::Event::MouseWheel { .. }
                )
            }),
        )
    });
    let id = pending_name_click_id(scope);
    let pending = ctx.data(|data| data.get_temp::<PendingNameClick>(id));
    if pending.is_some_and(|pending| {
        secondary_clicked
            || click_pos.is_some_and(|click_pos| !pending.rect.contains(click_pos))
            || keyboard_or_scroll
            || selected_path.is_none_or(|selected_path| pending.path != selected_path)
    }) {
        ctx.data_mut(|data| data.remove::<PendingNameClick>(id));
    }
}

/// Starts rename after a selected item's name is clicked and the double-click
/// interval expires. A real double-click cancels the pending rename so the
/// existing open action can run.
pub(crate) fn selected_name_click_starts_rename(
    ui: &Ui,
    scope: Id,
    response: &Response,
    item_path: &Path,
    name_rect: Rect,
    is_selected: bool,
) -> bool {
    let ctx = ui.ctx();
    let id = pending_name_click_id(scope);
    let now = ui.input(|input| input.time);
    let pending = ctx.data(|data| data.get_temp::<PendingNameClick>(id));
    let click_pos = ui.input(|input| input.pointer.interact_pos());
    let clicked_on_name =
        response.clicked() && click_pos.is_some_and(|pos| name_rect.contains(pos));
    let delay = if clicked_on_name && is_selected {
        ctx.options(|options| options.input_options.max_double_click_delay.max(0.0))
    } else {
        0.0
    };
    let (pending, should_start_rename) = update_pending_name_click(
        pending,
        item_path,
        name_rect,
        NameClickFlags {
            is_selected,
            clicked_on_name,
            double_clicked: response.double_clicked() || response.triple_clicked(),
        },
        now,
        delay,
    );
    ctx.data_mut(|data| {
        if let Some(pending) = pending.as_ref() {
            data.insert_temp(id, pending.clone());
        } else {
            data.remove::<PendingNameClick>(id);
        }
    });

    if should_start_rename {
        return true;
    }
    if let Some(pending) = pending {
        let remaining = (pending.deadline - now).max(0.0);
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(remaining));
    }
    false
}

#[cfg(test)]
mod tests {
    use super::{update_pending_name_click, NameClickFlags, PendingNameClick};
    use eframe::egui::{Pos2, Rect, Vec2};
    use std::path::Path;

    #[test]
    fn selected_name_click_waits_for_double_click_interval() {
        let path = Path::new("file.txt");
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::splat(10.0));
        let (pending, started) = update_pending_name_click(
            None,
            path,
            rect,
            NameClickFlags {
                is_selected: true,
                clicked_on_name: true,
                double_clicked: false,
            },
            1.0,
            0.3,
        );
        assert!(!started);
        assert_eq!(pending.as_ref().map(|pending| pending.deadline), Some(1.3));

        let (pending, started) = update_pending_name_click(
            pending,
            path,
            rect,
            NameClickFlags {
                is_selected: true,
                clicked_on_name: false,
                double_clicked: false,
            },
            1.2,
            0.3,
        );
        assert!(!started);
        assert!(pending.is_some());

        let (pending, started) = update_pending_name_click(
            pending,
            path,
            rect,
            NameClickFlags {
                is_selected: true,
                clicked_on_name: false,
                double_clicked: false,
            },
            1.31,
            0.3,
        );
        assert!(started);
        assert!(pending.is_none());
    }

    #[test]
    fn double_click_cancels_pending_rename() {
        let path = Path::new("file.txt");
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::splat(10.0));
        let pending = Some(PendingNameClick {
            path: path.to_path_buf(),
            rect,
            deadline: 1.3,
        });
        let (pending, started) = update_pending_name_click(
            pending,
            path,
            rect,
            NameClickFlags {
                is_selected: true,
                clicked_on_name: true,
                double_clicked: true,
            },
            1.1,
            0.3,
        );
        assert!(!started);
        assert!(pending.is_none());
    }
}
