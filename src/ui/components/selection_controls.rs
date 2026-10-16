use crate::ui::svg_icons::SvgIconManager;
use crate::ui::theme;
use eframe::egui;
use rust_i18n::t;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SelectionControlAction {
    SelectAll,
    Clear,
    Invert,
}

const BUTTON_SIZE: egui::Vec2 = egui::vec2(28.0, 24.0);
const BUTTON_SPACING: f32 = 4.0;
const GROUP_WIDTH: f32 = BUTTON_SIZE.x * 3.0 + BUTTON_SPACING * 2.0;

/// Draws the selection controls in a fixed-width region centered in the status bar.
pub(crate) fn render(
    ui: &mut egui::Ui,
    bar_rect: egui::Rect,
    svg_manager: &mut SvgIconManager,
    current_item_count: usize,
    has_selection: bool,
) -> Option<SelectionControlAction> {
    let controls_rect =
        egui::Rect::from_center_size(bar_rect.center(), egui::vec2(GROUP_WIDTH, BUTTON_SIZE.y));
    let mut controls_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(controls_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    controls_ui.spacing_mut().item_spacing.x = BUTTON_SPACING;

    let select_all = selection_button(
        &mut controls_ui,
        svg_manager,
        "select_all",
        &t!("status_bar.select_all"),
        current_item_count > 0,
    );
    let clear = selection_button(
        &mut controls_ui,
        svg_manager,
        "select_none",
        &t!("status_bar.select_none"),
        has_selection,
    );
    let invert = selection_button(
        &mut controls_ui,
        svg_manager,
        "invert_selection",
        &t!("status_bar.invert_selection"),
        current_item_count > 0,
    );

    if select_all {
        Some(SelectionControlAction::SelectAll)
    } else if clear {
        Some(SelectionControlAction::Clear)
    } else if invert {
        Some(SelectionControlAction::Invert)
    } else {
        None
    }
}

fn selection_button(
    ui: &mut egui::Ui,
    svg_manager: &mut SvgIconManager,
    icon_name: &str,
    tooltip: &str,
    enabled: bool,
) -> bool {
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(BUTTON_SIZE, sense);

    if enabled && response.hovered() {
        let hover_color = if ui.visuals().dark_mode {
            theme::color_dark_hover()
        } else {
            theme::color_hover()
        };
        ui.painter().rect_filled(rect, 4.0, hover_color);
    }

    let color = if enabled {
        if ui.visuals().dark_mode {
            [220, 220, 220, 255]
        } else {
            [60, 60, 60, 255]
        }
    } else {
        [128, 128, 128, 150]
    };
    if let Some(texture) = svg_manager.get_icon(ui.ctx(), icon_name, 32, color) {
        let icon_rect = egui::Rect::from_center_size(rect.center(), egui::vec2(16.0, 16.0));
        ui.painter().image(
            texture.id(),
            icon_rect,
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    }

    let response = if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    };
    response.on_hover_text(tooltip).clicked() && enabled
}
