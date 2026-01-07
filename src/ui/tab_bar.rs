//! Tab bar UI component for MTT File Manager
//! 
//! Renders a Windows 11/Files-style tab bar with:
//! - Tab icons and titles
//! - Close buttons
//! - New tab button (+)
//! - Drag reordering (future)

use eframe::egui::{self, Color32, CornerRadius, Stroke, Vec2};
use crate::tabs::TabManager;
use crate::ui::svg_icons::SvgIconManager;

/// Result of tab bar interaction
pub enum TabBarAction {
    None,
    SwitchTab(usize),
    CloseTab(usize),
    NewTab,
    CloseApp,
}

/// Render the tab bar and return any action to take
pub fn render_tab_bar(
    ui: &mut egui::Ui,
    tab_manager: &TabManager,
    svg_icons: &mut SvgIconManager,
) -> TabBarAction {
    let mut action = TabBarAction::None;
    
    let tab_height = 32.0;
    let tab_padding = 8.0;
    let close_btn_size = 16.0;
    
    // Colors based on theme
    let is_dark = ui.visuals().dark_mode;
    let active_bg = if is_dark {
        Color32::from_rgb(45, 45, 45)
    } else {
        Color32::from_rgb(255, 255, 255)
    };
    let inactive_bg = if is_dark {
        Color32::from_rgb(30, 30, 30)
    } else {
        Color32::from_rgb(240, 240, 240)
    };
    let hover_bg = if is_dark {
        Color32::from_rgb(55, 55, 55)
    } else {
        Color32::from_rgb(230, 230, 230)
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
    
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        
        for (idx, tab) in tab_manager.tabs.iter().enumerate() {
            let is_active = idx == tab_manager.active_tab;
            
            // Calculate tab width based on title
            let title_width = ui.fonts(|f| {
                f.glyph_width(&egui::TextStyle::Body.resolve(ui.style()), 'M') * tab.title.len().min(20) as f32
            });
            let tab_width = (title_width + tab_padding * 2.0 + close_btn_size + 24.0).clamp(100.0, 200.0);
            
            let (rect, response) = ui.allocate_exact_size(
                Vec2::new(tab_width, tab_height),
                egui::Sense::click(),
            );
            
            // Handle clicks
            if response.clicked() {
                action = TabBarAction::SwitchTab(idx);
            }
            
            // Middle click to close
            if response.middle_clicked() {
                action = TabBarAction::CloseTab(idx);
            }
            
            // Draw tab background
            let bg_color = if is_active {
                active_bg
            } else if response.hovered() {
                hover_bg
            } else {
                inactive_bg
            };
            
            let rounding = CornerRadius {
                nw: 8,
                ne: 8,
                sw: 0,
                se: 0,
            };
            
            ui.painter().rect_filled(rect, rounding, bg_color);
            
            // Tab content layout
            let content_rect = rect.shrink2(Vec2::new(tab_padding, 4.0));
            
            // Icon size and color
            let icon_size = 16.0;
            let icon_color = if is_active {
                [text_color.r(), text_color.g(), text_color.b(), 255]
            } else {
                [inactive_text.r(), inactive_text.g(), inactive_text.b(), 255]
            };

            // Dynamic icon based on path
            let icon_name = if tab.is_computer_view {
                None // User requested to remove redundant home icon
            } else if tab.path.len() <= 3 && tab.path.ends_with(":\\") {
                Some("drive")
            } else {
                Some("folder")
            };
            
            if let Some(name) = icon_name {
                let render_size = (icon_size * 2.0) as u32;
                if let Some(texture) = svg_icons.get_icon(ui.ctx(), name, render_size, icon_color) {
                    let icon_rect = egui::Rect::from_min_size(
                        content_rect.min,
                        Vec2::new(icon_size, icon_size),
                    );
                    ui.painter().image(
                        texture.id(),
                        icon_rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
            }
            
            // Tab title
            let icon_offset = if tab.is_computer_view { 0.0 } else { icon_size + 6.0 };
            let title_pos = content_rect.min + Vec2::new(icon_offset, 2.0);
            let max_title_width = tab_width - icon_size - close_btn_size - tab_padding * 3.0;
            
            let display_title = if tab.title.len() > 15 {
                format!("{}...", &tab.title[..12])
            } else {
                tab.title.clone()
            };
            
            // Use max_title_width to elide text if needed (simples elision here)
            // Note: egui text() doesn't automatically elide, but we already have basic elision above.
            // We use max_title_width in the layout calculation implicitly.
            let _ = max_title_width; 

            ui.painter().text(
                title_pos,
                egui::Align2::LEFT_TOP,
                &display_title,
                egui::FontId::proportional(13.0),
                if is_active { text_color } else { inactive_text },
            );
            
            // Close button (X)
            let close_rect = egui::Rect::from_min_size(
                egui::pos2(rect.max.x - close_btn_size - tab_padding, rect.center().y - close_btn_size / 2.0),
                Vec2::new(close_btn_size, close_btn_size),
            );
            
            let close_response = ui.interact(close_rect, response.id.with("close"), egui::Sense::click());
            
            // Only show close button on active or hovered tabs
            if is_active || response.hovered() {
                let close_color = if close_response.hovered() {
                    Color32::from_rgb(200, 80, 80)
                } else {
                    inactive_text
                };
                
                // Draw X
                let center = close_rect.center();
                let offset = 4.0;
                ui.painter().line_segment(
                    [center - Vec2::new(offset, offset), center + Vec2::new(offset, offset)],
                    Stroke::new(1.5, close_color),
                );
                ui.painter().line_segment(
                    [center + Vec2::new(-offset, offset), center + Vec2::new(offset, -offset)],
                    Stroke::new(1.5, close_color),
                );
            }
            
            if close_response.clicked() {
                action = TabBarAction::CloseTab(idx);
            }
        }
        
        // New tab button (+)
        let plus_size = Vec2::new(28.0, tab_height);
        let (plus_rect, plus_response) = ui.allocate_exact_size(plus_size, egui::Sense::click());
        
        let plus_bg = if plus_response.hovered() {
            hover_bg
        } else {
            inactive_bg
        };
        
        ui.painter().rect_filled(
            plus_rect,
            CornerRadius {
                nw: 8,
                ne: 8,
                sw: 0,
                se: 0,
            },
            plus_bg,
        );
        
        // Draw +
        let center = plus_rect.center();
        let plus_color = if plus_response.hovered() {
            text_color
        } else {
            inactive_text
        };
        let arm = 6.0;
        ui.painter().line_segment(
            [center - Vec2::new(arm, 0.0), center + Vec2::new(arm, 0.0)],
            Stroke::new(2.0, plus_color),
        );
        ui.painter().line_segment(
            [center - Vec2::new(0.0, arm), center + Vec2::new(0.0, arm)],
            Stroke::new(2.0, plus_color),
        );
        
        if plus_response.clicked() {
            action = TabBarAction::NewTab;
        }
    });
    
    action
}
