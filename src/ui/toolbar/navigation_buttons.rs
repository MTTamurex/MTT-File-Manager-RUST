use super::{theme, widgets, SvgIconManager, ToolbarAction};
use eframe::egui;
use rust_i18n::t;

pub(super) fn render(
    ui: &mut egui::Ui,
    svg_manager: &mut SvgIconManager,
    available: [bool; 3],
    is_renaming: bool,
) -> Option<ToolbarAction> {
    let mut action = None;
    for (enabled, icon, tooltip, command) in [
        (
            available[0],
            theme::ICON_ARROW_LEFT,
            t!("toolbar.back"),
            ToolbarAction::GoBack,
        ),
        (
            available[1],
            theme::ICON_ARROW_RIGHT,
            t!("toolbar.forward"),
            ToolbarAction::GoForward,
        ),
        (
            available[2],
            theme::ICON_ARROW_UP,
            t!("toolbar.up"),
            ToolbarAction::GoUp,
        ),
    ] {
        let response = ui
            .add_enabled_ui(enabled && !is_renaming, |ui| {
                widgets::icon_button(ui, svg_manager, icon, &tooltip, None)
            })
            .inner;
        if response.clicked() {
            action = Some(command);
        }
    }
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_navigation_buttons_never_emit_actions() {
        for available in [[false; 3], [true; 3]] {
            for renaming in [false, true] {
                for index in 0..3 {
                    let ctx = egui::Context::default();
                    let mut svg = SvgIconManager::default();
                    let point = std::cell::Cell::new(egui::Pos2::ZERO);
                    let mut action = None;
                    let mut render_ui = |ui: &mut egui::Ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            let size = theme::ICON_SIZE_MD + theme::PADDING_SM * 2.0;
                            point.set(
                                ui.cursor().min
                                    + egui::vec2(
                                        (size + 8.0) * index as f32 + size * 0.5,
                                        size * 0.5,
                                    ),
                            );
                            if let Some(command) = render(ui, &mut svg, available, renaming) {
                                action = Some(command);
                            }
                        });
                    };
                    let _ = ctx.run_ui(egui::RawInput::default(), &mut render_ui);
                    let click_point = point.get();
                    for pressed in [true, false] {
                        let _ = ctx.run_ui(
                            egui::RawInput {
                                events: vec![
                                    egui::Event::PointerMoved(click_point),
                                    egui::Event::PointerButton {
                                        pos: click_point,
                                        button: egui::PointerButton::Primary,
                                        pressed,
                                        modifiers: egui::Modifiers::NONE,
                                    },
                                ],
                                ..Default::default()
                            },
                            &mut render_ui,
                        );
                    }
                    assert_eq!(action.is_some(), available[index] && !renaming);
                    if let Some(action) = action {
                        assert!(matches!(
                            (index, action),
                            (0, ToolbarAction::GoBack)
                                | (1, ToolbarAction::GoForward)
                                | (2, ToolbarAction::GoUp)
                        ));
                    }
                }
            }
        }
    }
}
