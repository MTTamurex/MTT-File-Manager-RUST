use super::TabBarAction;
use eframe::egui::{self, Color32, Stroke};

pub(super) fn finish_drag(
    ui: &egui::Ui,
    source: Option<usize>,
    rects: &[egui::Rect],
    cancel: bool,
    pointer: Option<egui::Pos2>,
    released: bool,
) -> TabBarAction {
    let Some(source) = source else {
        return TabBarAction::None;
    };
    let drag_id = egui::Id::new("tab_reorder_drag");
    let clear = || ui.ctx().data_mut(|data| data.remove::<usize>(drag_id));
    if source >= rects.len() {
        clear();
        return TabBarAction::None;
    }
    if cancel {
        clear();
        return TabBarAction::None;
    }
    if super::detach::released_outside(ui.ctx()) {
        clear();
        return TabBarAction::DetachTab(source);
    }
    let Some(pointer) = pointer else {
        if released {
            clear();
        }
        return TabBarAction::None;
    };
    let row = rects[0].union(rects[rects.len() - 1]);
    let in_row = row.contains(pointer);
    let gap = rects
        .iter()
        .position(|rect| pointer.x < rect.center().x)
        .unwrap_or(rects.len());
    if released {
        clear();
        if in_row {
            return TabBarAction::MoveTab { from: source, gap };
        }
    } else {
        ui.ctx().request_repaint();
        if in_row {
            let x = rects.get(gap).map_or(row.right(), |rect| rect.left());
            ui.painter().vline(
                x,
                row.y_range(),
                Stroke::new(2.0, Color32::from_rgb(0, 120, 215)),
            );
        }
    }
    TabBarAction::None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drop_at(position: egui::Pos2, escape: bool) -> TabBarAction {
        drop_with_cancel(position, escape, escape)
    }

    fn drop_with_cancel(position: egui::Pos2, escape: bool, cancel: bool) -> TabBarAction {
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let button = |pressed| egui::Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let _ = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(screen),
                events: vec![button(true)],
                ..Default::default()
            },
            |_| {},
        );
        let mut events = vec![button(false)];
        if escape {
            events.push(egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
        }
        let mut action = TabBarAction::None;
        let _ = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(screen),
                events,
                ..Default::default()
            },
            |ui| {
                let tabs = [egui::Rect::from_min_size(
                    egui::pos2(0.0, 0.0),
                    egui::vec2(200.0, 36.0),
                )];
                action = finish_drag(ui, Some(0), &tabs, cancel, Some(position), true);
            },
        );
        action
    }

    #[test]
    fn outer_tab_detaches_outside_but_not_over_window_content() {
        assert!(matches!(
            drop_at(egui::pos2(900.0, 100.0), false),
            TabBarAction::DetachTab(0)
        ));
        assert!(matches!(
            drop_at(egui::pos2(100.0, 100.0), false),
            TabBarAction::None
        ));
        assert!(matches!(
            drop_at(egui::pos2(900.0, 100.0), true),
            TabBarAction::None
        ));
        assert!(matches!(
            drop_at(egui::pos2(100.0, 18.0), false),
            TabBarAction::MoveTab { from: 0, .. }
        ));
    }

    #[test]
    fn cancelled_or_unowned_drag_cannot_detach_even_with_native_release() {
        assert!(matches!(
            drop_with_cancel(egui::pos2(900.0, 100.0), false, true),
            TabBarAction::None
        ));
    }
}
