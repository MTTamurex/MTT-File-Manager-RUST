use crate::ui::svg_icons::SvgIconManager;
use crate::ui::theme::{self, *};
use eframe::egui;

/// Renders an icon button with SVG support and optional texture override
pub fn icon_button(
    ui: &mut egui::Ui,
    svg_manager: &mut SvgIconManager,
    icon: &str,
    tooltip: &str,
    texture_override: Option<&egui::TextureHandle>,
) -> egui::Response {
    // Map unicode icons to SVG names
    let icon_name = match icon {
        ICON_ARROW_LEFT => "nav_back",
        ICON_ARROW_RIGHT => "nav_forward",
        ICON_ARROW_UP => "nav_up",
        ICON_REFRESH => "refresh",
        ICON_HOME => "home",
        ICON_SEARCH => "search",
        ICON_FOLDER_ADD => "folder_new",
        _ => icon,
    };

    let size = theme::ICON_SIZE_MD;
    let padding = theme::PADDING_SM;
    let button_size = egui::vec2(size + padding * 2.0, size + padding * 2.0);

    let (rect, response) = ui.allocate_exact_size(button_size, egui::Sense::click());

    // Hover Effect - Paint background if hovered
    if response.hovered() {
        let bg_color = if ui.visuals().dark_mode {
            theme::color_dark_hover()
        } else {
            theme::color_hover()
        };
        ui.painter().rect_filled(rect, theme::PADDING_SM, bg_color);
    }

    // Cursor
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);

    // 1. Texture Override (Essential for Home/Computer icon)
    // Always render this if provided, ignoring SVG lookup
    if let Some(texture) = texture_override {
        let icon_rect = egui::Rect::from_center_size(
            rect.center(),
            egui::vec2(theme::ICON_SIZE_MD, theme::ICON_SIZE_MD),
        );
        ui.painter().image(
            texture.id(),
            icon_rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
        return response.on_hover_text(tooltip);
    }

    // 2. SVG Icon
    // Determine icon color
    let color = if ui.visuals().dark_mode {
        [220, 220, 220, 255]
    } else {
        [60, 60, 60, 255]
    };

    if let Some(texture) =
        svg_manager.get_icon(ui.ctx(), icon_name, theme::ICON_SIZE_MD as u32, color)
    {
        let icon_rect = egui::Rect::from_center_size(
            rect.center(),
            egui::vec2(theme::ICON_SIZE_MD, theme::ICON_SIZE_MD),
        );
        ui.painter().image(
            texture.id(),
            icon_rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
        return response.on_hover_text(tooltip);
    }

    // 3. Fallback: Text/Emoji
    // Use direct painting or non-interactive Label to avoid stealing clicks/hover
    let text_color = if ui.visuals().dark_mode {
        egui::Color32::from_rgb(200, 200, 200)
    } else {
        egui::Color32::from_rgb(60, 60, 60)
    };

    // We can just paint text centered
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icon,
        egui::FontId::proportional(theme::ICON_SIZE_MD),
        text_color,
    );

    response.on_hover_text(tooltip)
}

/// Renders a toggle button that shows active/inactive state
pub fn toggle_icon_button(
    ui: &mut egui::Ui,
    svg_manager: &mut SvgIconManager,
    icon: &str,
    active: bool,
    tooltip: &str,
) -> egui::Response {
    toggle_icon_button_sized(
        ui,
        svg_manager,
        icon,
        active,
        tooltip,
        theme::ICON_SIZE_LG,
        theme::PADDING_SM,
        0.0,
    )
}

/// Renders a toggle button with configurable icon size/padding.
#[allow(clippy::too_many_arguments)]
pub fn toggle_icon_button_sized(
    ui: &mut egui::Ui,
    svg_manager: &mut SvgIconManager,
    icon: &str,
    active: bool,
    tooltip: &str,
    size: f32,
    padding: f32,
    icon_offset_y: f32,
) -> egui::Response {
    let icon_name = match icon {
        ICON_GRID => "view_grid",
        ICON_LIST => "view_list",
        ICON_DETAILS => "info",
        _ => icon,
    };

    let button_size = egui::vec2(size + padding * 2.0, size + padding * 2.0);

    let (rect, response) = ui.allocate_exact_size(button_size, egui::Sense::click());

    // Cursor
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);

    // Hover Bg
    if response.hovered() {
        let bg_color = if ui.visuals().dark_mode {
            theme::color_dark_hover()
        } else {
            theme::color_hover()
        };
        ui.painter().rect_filled(rect, theme::PADDING_SM, bg_color);
    }

    // Active State Colors
    let color = if active {
        [0, 120, 215, 255] // Blue for active
    } else if ui.visuals().dark_mode {
        [220, 220, 220, 255]
    } else {
        [60, 60, 60, 255]
    };

    if let Some(texture) = svg_manager.get_icon(ui.ctx(), icon_name, size as u32, color) {
        let icon_center = egui::pos2(rect.center().x, rect.center().y + icon_offset_y);
        let icon_rect = egui::Rect::from_center_size(icon_center, egui::vec2(size, size));
        ui.painter().image(
            texture.id(),
            icon_rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
        return response.on_hover_text(tooltip);
    }

    // Fallback
    let text_color = if active {
        theme::COLOR_ACCENT
    } else {
        ui.visuals().text_color()
    };

    // Direct text painting for fallback
    ui.painter().text(
        egui::pos2(rect.center().x, rect.center().y + icon_offset_y),
        egui::Align2::CENTER_CENTER,
        icon,
        egui::FontId::proportional(theme::ICON_SIZE_MD),
        text_color,
    );

    response.on_hover_text(tooltip)
}
