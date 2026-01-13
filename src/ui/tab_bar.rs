//! Tab bar UI component for MTT File Manager
//! 
//! Renders a Windows 11/Files-style tab bar with:
//! - Tab icons and titles
//! - Close buttons
//! - New tab button (+)
//! - Window controls (minimize, maximize, close)
//! - Custom title bar with drag area
//! - Auto-resizing tabs

use eframe::egui::{self, Color32, CornerRadius, Stroke, Vec2};
use crate::tabs::TabManager;
use crate::ui::svg_icons::SvgIconManager;
use crate::ui::icon_loader::IconLoader;

/// Result of tab bar interaction
pub enum TabBarAction {
    None,
    SwitchTab(usize),
    CloseTab(usize),
    NewTab,
    CloseApp,
    ToggleMaximize,
    Minimize,
}

/// Render the tab bar with custom title bar (Windows 11 style)
pub fn render_tab_bar(
    ui: &mut egui::Ui,
    tab_manager: &TabManager,
    svg_icons: &mut SvgIconManager,
    frame: &mut eframe::Frame,
    computer_icon: Option<&egui::TextureHandle>,
    icon_loader: &mut IconLoader,
) -> TabBarAction {
    let ctx = ui.ctx().clone();
    let mut action = TabBarAction::None;
    
    let tab_height = 36.0; // Slightly taller for integrated title bar
    let tab_padding = 8.0;
    let close_btn_size = 16.0;
    let window_btn_width = 46.0; // Standard Windows button width
    let window_controls_width = window_btn_width * 3.0; // Min, Max, Close
    
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
        for (idx, tab) in tab_manager.tabs.iter().enumerate() {
            let is_active = idx == tab_manager.active_tab;
            
            
            let (rect, response) = ui.allocate_exact_size(
                Vec2::new(ideal_tab_width, tab_height),
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
            
            // Background
            let bg_color = if is_active {
                active_bg
            } else if response.hovered() {
                hover_bg
            } else {
                inactive_bg
            };
            
            ui.painter().rect_filled(
                rect,
                CornerRadius {
                    nw: 6,
                    ne: 6,
                    sw: 0,
                    se: 0,
                },
                bg_color,
            );
            
            // Subtle active indicator (removed thick blue line)
            // Active tab is already distinguished by lighter background
            
            // Content area (icon + text + close button)
            let content_rect = rect.shrink2(Vec2::new(tab_padding, 4.0));
            
            // Dynamic icon (computer for "Este Computador", folder for regular paths)
            let icon_size = 16.0;
            let icon_pos = content_rect.min + Vec2::new(0.0, (content_rect.height() - icon_size) / 2.0);
            let icon_rect = egui::Rect::from_min_size(icon_pos, Vec2::splat(icon_size));
            
            // Render at 2x resolution for HiDPI clarity
            let render_size = 32;
            let icon_name = if tab.is_computer_view { "home" } else { "folder" };
            // Dark blue/gray color that's clearly visible
            let icon_color = if is_active { 
                [30, 90, 180, 255] // Dark blue for active
            } else { 
                [80, 80, 80, 255] // Dark gray for inactive
            };
            
            let native_icon = if tab.is_computer_view {
                computer_icon.cloned()
            } else if tab.path == "Lixeira" {
                icon_loader.ensure_recycle_bin_icon(ui.ctx())
            } else {
                icon_loader.get_or_load_folder_path_icon(ui.ctx(), &tab.path)
            };

            if let Some(texture) = native_icon {
                ui.painter().image(
                    texture.id(),
                    icon_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            } else if let Some(texture) = svg_icons.get_icon(ui.ctx(), icon_name, render_size, icon_color) {
                ui.painter().image(
                    texture.id(),
                    icon_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            
            // Tab title (truncated dynamically based on available width)
            let title_x = icon_pos.x + icon_size + 6.0;
            let title_max_width = ideal_tab_width - icon_size - close_btn_size - tab_padding * 2.0 - 12.0;
            
            // Use egui's galley to measure text and truncate properly
            let font_id = egui::FontId::proportional(13.0);
            let title_color = if is_active { text_color } else { inactive_text };
            
            let full_text = tab.title.clone();
            let mut title_text = full_text.clone();
            
            // Measure text width and truncate if needed
            let galley = ui.painter().layout_no_wrap(
                title_text.clone(),
                font_id.clone(),
                title_color,
            );
            
            if galley.rect.width() > title_max_width {
                // Binary search for the right truncation point
                let mut low = 0;
                let mut high = full_text.len();
                
                while low < high {
                    let mid = (low + high + 1) / 2;
                    let test_text = format!("{}...", &full_text[..mid.min(full_text.len())]);
                    let test_galley = ui.painter().layout_no_wrap(
                        test_text.clone(),
                        font_id.clone(),
                        title_color,
                    );
                    
                    if test_galley.rect.width() <= title_max_width {
                        low = mid;
                    } else {
                        high = mid - 1;
                    }
                }
                
                if low > 0 {
                    title_text = format!("{}...", &full_text[..low.min(full_text.len())]);
                } else {
                    title_text = "...".to_string();
                }
            }
            
            ui.painter().text(
                egui::pos2(title_x, content_rect.center().y),
                egui::Align2::LEFT_CENTER,
                title_text,
                font_id,
                title_color,
            );
            
            // Close button
            let close_btn_x = rect.max.x - close_btn_size - tab_padding;
            let close_btn_y = content_rect.center().y - close_btn_size / 2.0;
            let close_btn_rect = egui::Rect::from_min_size(
                egui::pos2(close_btn_x, close_btn_y),
                Vec2::splat(close_btn_size),
            );
            
            let close_response = ui.interact(close_btn_rect, egui::Id::new(format!("close_{}", idx)), egui::Sense::click());
            
            if close_response.clicked() {
                action = TabBarAction::CloseTab(idx);
            }
            
            // Close button background on hover
            if close_response.hovered() {
                ui.painter().rect_filled(
                    close_btn_rect,
                    2.0,
                    Color32::from_rgba_unmultiplied(255, 255, 255, 30),
                );
            }
            
            // X icon
            let x_stroke = Stroke::new(1.5, if close_response.hovered() { Color32::WHITE } else { inactive_text });
            let x_center = close_btn_rect.center();
            let x_radius = close_btn_size * 0.25;
            ui.painter().line_segment(
                [
                    x_center + Vec2::new(-x_radius, -x_radius),
                    x_center + Vec2::new(x_radius, x_radius),
                ],
                x_stroke,
            );
            ui.painter().line_segment(
                [
                    x_center + Vec2::new(x_radius, -x_radius),
                    x_center + Vec2::new(-x_radius, x_radius),
                ],
                x_stroke,
            );
        }
        
        // New tab button (+)
        let new_tab_btn_width = 36.0;
        let (new_tab_rect, new_tab_response) = ui.allocate_exact_size(
            Vec2::new(new_tab_btn_width, tab_height),
            egui::Sense::click(),
        );
        
        if new_tab_response.clicked() {
            action = TabBarAction::NewTab;
        }        
        // Drag area (space between new tab button and window controls)
        let remaining_width = ui.available_width() - window_controls_width;
        if remaining_width > 0.0 {
            let (drag_rect, drag_response) = ui.allocate_exact_size(
                Vec2::new(remaining_width, tab_height),
                egui::Sense::click_and_drag(),  // Mudado para click_and_drag
            );
            
            // Drag to move window
            if drag_response.drag_started() || drag_response.dragged() {
                ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
        }        
        let new_tab_bg = if new_tab_response.hovered() {
            hover_bg
        } else {
            inactive_bg
        };
        
        ui.painter().rect_filled(
            new_tab_rect,
            CornerRadius {
                nw: 6,
                ne: 6,
                sw: 0,
                se: 0,
            },
            new_tab_bg,
        );
        
        // + icon
        let plus_center = new_tab_rect.center();
        let plus_size = 10.0;
        let plus_stroke = Stroke::new(2.0, text_color);
        ui.painter().line_segment(
            [
                plus_center + Vec2::new(-plus_size / 2.0, 0.0),
                plus_center + Vec2::new(plus_size / 2.0, 0.0),
            ],
            plus_stroke,
        );
        ui.painter().line_segment(
            [
                plus_center + Vec2::new(0.0, -plus_size / 2.0),
                plus_center + Vec2::new(0.0, plus_size / 2.0),
            ],
            plus_stroke,
        );
        
        // Push window controls to the right
        ui.add_space(ui.available_width() - window_controls_width);
        
        // NOTE: Window controls (min/max/close) are now provided by native Windows decorations
        // render_window_controls(ui, frame, window_btn_width, tab_height, &mut action);
    });
    
    action
}

/// Render window control buttons (Minimize, Maximize, Close)
fn render_window_controls(
    ui: &mut egui::Ui,
    frame: &mut eframe::Frame,
    btn_width: f32,
    btn_height: f32,
    action: &mut TabBarAction,
) {
    let is_dark = ui.visuals().dark_mode;
    let text_color = if is_dark {
        Color32::from_rgb(220, 220, 220)
    } else {
        Color32::from_rgb(30, 30, 30)
    };
    let hover_bg = if is_dark {
        Color32::from_rgb(60, 60, 60)
    } else {
        Color32::from_rgb(230, 230, 230)
    };
    let close_hover_bg = Color32::from_rgb(232, 17, 35); // Windows red
    
    ui.spacing_mut().item_spacing.x = 0.0;
    
    // Check if maximized
    let is_maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
    
    // Minimize button
    let (min_rect, min_response) = ui.allocate_exact_size(
        Vec2::new(btn_width, btn_height),
        egui::Sense::click(),
    );
    
    if min_response.clicked() {
        *action = TabBarAction::Minimize;
    }
    
    if min_response.hovered() {
        ui.painter().rect_filled(min_rect, 0.0, hover_bg);
    }
    
    // Minimize icon (horizontal line)
    let min_center = min_rect.center();
    ui.painter().line_segment(
        [
            min_center + Vec2::new(-5.0, 0.0),
            min_center + Vec2::new(5.0, 0.0),
        ],
        Stroke::new(1.0, text_color),
    );
    
    // Maximize/Restore button
    let (max_rect, max_response) = ui.allocate_exact_size(
        Vec2::new(btn_width, btn_height),
        egui::Sense::click(),
    );
    
    if max_response.clicked() {
        *action = TabBarAction::ToggleMaximize;
    }
    
    if max_response.hovered() {
        ui.painter().rect_filled(max_rect, 0.0, hover_bg);
    }
    
    // Maximize/Restore icon
    let max_center = max_rect.center();
    let icon_size = 10.0;
    
    if is_maximized {
        // Restore icon (two overlapping squares)
        let offset = 2.0;
        // Back square
        ui.painter().rect_stroke(
            egui::Rect::from_center_size(
                max_center + Vec2::new(-offset, offset),
                Vec2::splat(icon_size - 2.0),
            ),
            0.0,
            Stroke::new(1.0, text_color),
            egui::StrokeKind::Inside,
        );
        // Front square
        ui.painter().rect_stroke(
            egui::Rect::from_center_size(
                max_center + Vec2::new(offset, -offset),
                Vec2::splat(icon_size - 2.0),
            ),
            0.0,
            Stroke::new(1.0, text_color),
            egui::StrokeKind::Inside,
        );
    } else {
        // Maximize icon (single square)
        ui.painter().rect_stroke(
            egui::Rect::from_center_size(max_center, Vec2::splat(icon_size)),
            0.0,
            Stroke::new(1.0, text_color),            egui::StrokeKind::Inside,        );
    }
    
    // Close button
    let (close_rect, close_response) = ui.allocate_exact_size(
        Vec2::new(btn_width, btn_height),
        egui::Sense::click(),
    );
    
    if close_response.clicked() {
        *action = TabBarAction::CloseApp;
    }
    
    if close_response.hovered() {
        ui.painter().rect_filled(close_rect, 0.0, close_hover_bg);
    }
    
    // Close icon (X)
    let close_center = close_rect.center();
    let x_size = 10.0;
    let x_color = if close_response.hovered() {
        Color32::WHITE
    } else {
        text_color
    };
    let x_stroke = Stroke::new(1.0, x_color);
    
    ui.painter().line_segment(
        [
            close_center + Vec2::new(-x_size / 2.0, -x_size / 2.0),
            close_center + Vec2::new(x_size / 2.0, x_size / 2.0),
        ],
        x_stroke,
    );
    ui.painter().line_segment(
        [
            close_center + Vec2::new(x_size / 2.0, -x_size / 2.0),
            close_center + Vec2::new(-x_size / 2.0, x_size / 2.0),
        ],
        x_stroke,
    );
}
