//! Tab bar UI component for MTT File Manager
//!
//! Renders a Windows 11/Files-style tab bar with:
//! - Tab icons and titles
//! - Close buttons
//! - New tab button (+)
//! - Window controls (minimize, maximize, close)
//! - Custom title bar with drag area
//! - Auto-resizing tabs

use crate::tabs::TabManager;
use crate::ui::icon_loader::IconLoader;
use crate::ui::svg_icons::SvgIconManager;
use crate::ui::theme;
use eframe::egui::{self, Color32};

mod drag_dwell;
mod new_tab_area;
mod tabs_renderer;
mod window_controls;

/// Result of tab bar interaction
pub enum TabBarAction {
    None,
    SwitchTab(usize),
    CloseTab(usize),
    NewTab,
    CloseApp,
    ToggleMaximize,
    Minimize,
    ToggleMute(usize), // Tab index
}

/// Render the tab bar with custom title bar (Windows 11 style)
pub fn render_tab_bar(
    ui: &mut egui::Ui,
    tab_manager: &TabManager,
    svg_icons: &mut SvgIconManager,
    _frame: &mut eframe::Frame,
    computer_icon: Option<&egui::TextureHandle>,
    icon_loader: &mut IconLoader,
    media_owner_id: Option<usize>,
    is_playing: bool,
    is_muted: bool,
    is_item_dragging: bool,
) -> TabBarAction {
    let ctx = ui.ctx().clone();
    let mut action = TabBarAction::None;

    let tab_height = 36.0; // Slightly taller for integrated title bar
    let tab_padding = 8.0;
    let close_btn_size = 16.0;
    let window_btn_width = 46.0; // Standard Windows button width
    let window_controls_width = window_btn_width * 3.0; // Min, Max, Close

    // Colors based on theme (Windows Explorer style - gray tones)
    let is_dark = ui.visuals().dark_mode;
    let active_bg = if is_dark {
        Color32::from_rgb(45, 45, 45)
    } else {
        Color32::from_rgb(243, 243, 243) // Light gray for active tab (Windows Explorer style)
    };
    let inactive_bg = if is_dark {
        Color32::from_rgb(30, 30, 30)
    } else {
        Color32::from_rgb(230, 230, 230) // Darker gray for inactive tabs
    };
    let hover_bg = if is_dark {
        theme::color_dark_hover()
    } else {
        theme::color_hover()
    };
    let text_color = if is_dark {
        Color32::from_rgb(220, 220, 220)
    } else {
        Color32::from_rgb(30, 30, 30)
    };
    let inactive_text = if is_dark {
        Color32::from_rgb(160, 160, 160)
    } else {
        Color32::from_rgb(100, 100, 100)
    };

    // Calculate available space for tabs
    let available_width = ui.available_width() - window_controls_width - 50.0; // 50.0 for new tab button
    let num_tabs = tab_manager.tabs.len();

    // Dynamic tab width calculation (Windows Explorer behavior)
    let min_tab_width = 100.0;
    let max_tab_width = 200.0;
    let ideal_tab_width = (available_width / num_tabs as f32).clamp(min_tab_width, max_tab_width);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0; // Remove spacing between tabs
                                               // Render tabs
        let tabs_action = tabs_renderer::render_tabs(
            ui,
            tab_manager,
            svg_icons,
            computer_icon,
            icon_loader,
            media_owner_id,
            is_playing,
            is_muted,
            is_item_dragging,
            ideal_tab_width,
            tab_height,
            tab_padding,
            close_btn_size,
            active_bg,
            inactive_bg,
            hover_bg,
            text_color,
            inactive_text,
            is_dark,
        );
        if !matches!(tabs_action, TabBarAction::None) {
            action = tabs_action;
        }

        if new_tab_area::render_new_tab_and_drag_area(
            ui,
            &ctx,
            tab_height,
            window_controls_width,
            inactive_bg,
            hover_bg,
            text_color,
        ) {
            action = TabBarAction::NewTab;
        }

        // Render window controls (min/max/close) for borderless window
        window_controls::render_window_controls(
            ui,
            _frame,
            window_btn_width,
            tab_height,
            &mut action,
        );
    });

    action
}
