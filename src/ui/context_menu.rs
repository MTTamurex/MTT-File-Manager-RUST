//! Context menu rendering (Files-style 1:1 clone)
//! Follows .cursorrules: single responsibility, < 300 lines

use eframe::egui::{self, Sense};
use std::cell::RefCell;

use crate::application::context_menu::{ContextMenuState, ContextMenuItem};

// Track submenu hierarchy: each depth level stores which item is active at that level
// Example: hovering "7-Zip" -> [Some(7zip_id)]
// Hovering item inside 7-Zip submenu -> [Some(7zip_id), Some(sub_item_id)]
thread_local! {
    static SUBMENU_HIERARCHY: RefCell<Vec<Option<i32>>> = RefCell::new(Vec::new());
}

/// Operations that can be performed from context menu
pub trait ContextMenuOperations {
    fn create_new_folder(&mut self);
    fn command_copy(&mut self);
    fn command_cut(&mut self);
    fn command_paste(&mut self);
    fn rename_item(&mut self, idx: usize);
    fn delete_with_shell(&mut self);
}

/// Menu styling constants (matching Files app - compact)
const HEADER_ICON_SIZE: f32 = 14.0;
const HEADER_BUTTON_SIZE: f32 = 24.0;
const HEADER_SPACING: f32 = 2.0;
const ITEM_HEIGHT: f32 = 22.0;
const ITEM_ICON_SIZE: f32 = 16.0;
const MENU_ROUNDING: f32 = 6.0;
const MENU_MIN_WIDTH: f32 = 180.0;
const MENU_MAX_WIDTH: f32 = 320.0;
const SUBMENU_MIN_WIDTH: f32 = 200.0;
const SUBMENU_X_OFFSET: f32 = 6.0;
const SHORTCUT_COLOR: egui::Color32 = egui::Color32::from_gray(128);
const TEXT_SHORTCUT_GAP: f32 = 24.0; // Gap between text and shortcut

/// Unicode icons for header bar (matching Files/Windows 11 style)
const ICON_CUT: &str = "✂";
const ICON_COPY: &str = "📋";
const ICON_PASTE: &str = "📄";
const ICON_RENAME: &str = "✏";
const ICON_DELETE: &str = "🗑";
const ICON_PROPERTIES: &str = "⚙";

/// Renders the Files-style context menu
pub fn render_context_menu(
    ctx: &egui::Context,
    menu_state: &mut ContextMenuState,
    _ops: &mut dyn ContextMenuOperations,
) -> bool {
    if !menu_state.is_open {
        return false;
    }

    let mut action_executed: Option<i32> = None;
    let mut should_close = false;

    // Separate primary (header) and secondary items
    let primary_items: Vec<&ContextMenuItem> = menu_state.items.iter()
        .filter(|i| i.is_primary && !i.is_separator)
        .collect();
    let secondary_items: Vec<&ContextMenuItem> = menu_state.items.iter()
        .filter(|i| !i.is_primary && !i.show_in_overflow)
        .collect();
    let overflow_items: Vec<&ContextMenuItem> = menu_state.items.iter()
        .filter(|i| i.show_in_overflow)
        .collect();

    // Render the menu popup
    let response = egui::Area::new(egui::Id::new("context_menu"))
        .fixed_pos(menu_state.position)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style())
                .inner_margin(4.0)
                .corner_radius(MENU_ROUNDING)
                .show(ui, |ui| {
                    ui.set_min_width(MENU_MIN_WIDTH); // Ensure a base width for alignment
                    ui.set_max_width(MENU_MAX_WIDTH); // Clamp width to avoid oversized menus
                    ui.spacing_mut().item_spacing = egui::vec2(0.0, 1.0);

                    // ========== HEADER BAR (Primary items as icons) ==========
                    if !primary_items.is_empty() {
                        render_header_bar(ui, &primary_items, &mut action_executed);
                        ui.separator();
                    }

                    // ========== SECONDARY ITEMS (Regular menu items) ==========
                    render_menu_items(ui, &secondary_items, &mut action_executed);

                    // ========== OVERFLOW ("Show more options") ==========
                    if !overflow_items.is_empty() {
                        render_overflow_submenu(ui, &overflow_items, &mut action_executed);
                    }
                });
        });

    // Handle action execution
    if let Some(id) = action_executed {
        menu_state.selected_command_id = Some(id);
        should_close = true;
    }

    // Close menu on left-click outside (use released to avoid capturing the opening click)
    if !should_close && ctx.input(|i| i.pointer.primary_released()) {
        if let Some(pos) = ctx.pointer_interact_pos() {
            if !response.response.rect.contains(pos) {
                should_close = true;
            }
        }
    }

    // Close on Escape
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        should_close = true;
    }

    if should_close {
        menu_state.is_open = false;
        if action_executed.is_none() {
            menu_state.close();
        }
        return true;
    }

    false
}

/// Render the header bar with primary action icons
fn render_header_bar(
    ui: &mut egui::Ui,
    items: &[&ContextMenuItem],
    action: &mut Option<i32>,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(HEADER_SPACING, 0.0);
        
        for item in items {
            let btn_size = egui::vec2(HEADER_BUTTON_SIZE, HEADER_BUTTON_SIZE);
            
            // Get icon based on command_string
            let icon_str = match item.command_string.as_deref() {
                Some("cut") => ICON_CUT,
                Some("copy") => ICON_COPY,
                Some("paste") => ICON_PASTE,
                Some("rename") => ICON_RENAME,
                Some("delete") => ICON_DELETE,
                Some("properties") => ICON_PROPERTIES,
                _ => "?",
            };
            
            let response = if let Some(icon) = &item.icon {
                // Use texture icon if available
                let img = egui::Image::from_texture(egui::load::SizedTexture::new(
                    icon.id(),
                    egui::vec2(HEADER_ICON_SIZE, HEADER_ICON_SIZE),
                ));
                ui.add_sized(btn_size, egui::ImageButton::new(img))
            } else {
                // Use Unicode icon
                let btn = egui::Button::new(egui::RichText::new(icon_str).size(12.0));
                ui.add_sized(btn_size, btn)
            };

            // Tooltip with item name and shortcut
            let tooltip = if let Some(shortcut) = &item.keyboard_shortcut {
                format!("{} ({})", item.text, shortcut)
            } else {
                item.text.clone()
            };
            let response = response.on_hover_text(tooltip);
            
            if response.clicked() && item.is_enabled {
                *action = Some(item.id);
            }
        }
    });
}

/// Render list of menu items with icons and keyboard shortcuts
fn render_menu_items(
    ui: &mut egui::Ui,
    items: &[&ContextMenuItem],
    action: &mut Option<i32>,
) {
    let mut last_was_separator = true; // collapse leading separators

    for item in items {
        if item.is_separator {
            if last_was_separator {
                continue; // skip duplicate/leading separators
            }
            render_single_item(ui, item, action, 0);  // Top-level items have depth 0
            last_was_separator = true;
        } else {
            render_single_item(ui, item, action, 0);  // Top-level items have depth 0
            last_was_separator = false;
        }
    }
}

/// Render a single menu item using egui's natural layout
fn render_single_item(
    ui: &mut egui::Ui,
    item: &ContextMenuItem,
    action: &mut Option<i32>,
    depth: usize,  // NEW: Track nesting depth for hierarchical state
) {
    if item.is_separator {
        ui.separator();
        return;
    }

    let has_submenu = !item.sub_items.is_empty();
    
    // Build the label with icon + text + shortcut/arrow
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ITEM_HEIGHT),
        Sense::click(),
    );
    
    // Hover highlight
    if response.hovered() {
        ui.painter().rect_filled(
            rect,
            3.0,
            ui.visuals().widgets.hovered.bg_fill,
        );
    }

    // Icon (16x16)
    let icon_rect = egui::Rect::from_min_size(
        egui::pos2(rect.min.x + 8.0, rect.center().y - ITEM_ICON_SIZE / 2.0),
        egui::vec2(ITEM_ICON_SIZE, ITEM_ICON_SIZE),
    );
    
    if let Some(icon) = &item.icon {
        let img = egui::Image::from_texture(egui::load::SizedTexture::new(
            icon.id(),
            icon_rect.size(),
        ));
        img.paint_at(ui, icon_rect);
    }
    
    // Text with ellipsis truncation to prevent overflow
    let text_x = icon_rect.right() + 8.0;
    let text_color = if item.is_enabled {
        ui.visuals().text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    
    // Truncate very long names (like drive paths in "Send to") with ellipsis
    let display_text = if item.text.len() > 32 {
        format!("{}…", &item.text[..30])
    } else {
        item.text.clone()
    };
    
    ui.painter().text(
        egui::pos2(text_x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        &display_text,
        egui::FontId::proportional(12.0),
        text_color,
    );
    
    // Right-aligned part (Shortcut or Arrow)
    let right_alignment_pos = rect.right() - 10.0;
    if has_submenu {
        ui.painter().text(
            egui::pos2(right_alignment_pos, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            "›",
            egui::FontId::proportional(14.0),
            text_color,
        );
    } else if let Some(shortcut) = &item.keyboard_shortcut {
        ui.painter().text(
            egui::pos2(right_alignment_pos, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            shortcut,
            egui::FontId::proportional(10.0),
            SHORTCUT_COLOR,
        );
    }

    // Handle click
    if response.clicked() && item.is_enabled && !has_submenu {
        *action = Some(item.id);
    }

    // Handle submenu on hover (keep open while cursor is over item OR submenu)
    if has_submenu {
        let pointer_pos = ui.ctx().pointer_latest_pos();
        let submenu_pos = egui::pos2(rect.right() + SUBMENU_X_OFFSET, rect.top());
        
        // Estimate submenu size for hover detection
        let estimated_height = item
            .sub_items
            .iter()
            .fold(8.0, |acc, sub| acc + if sub.is_separator { 6.0 } else { ITEM_HEIGHT + 1.0 });
        let estimated_submenu_rect = egui::Rect::from_min_size(
            submenu_pos,
            egui::vec2(MENU_MAX_WIDTH, estimated_height),
        );
        
        // Create a bridge zone that covers the ENTIRE submenu height
        // This allows cursor to move up/down freely within the submenu without closing it
        let bridge_rect = egui::Rect::from_min_max(
            egui::pos2(rect.right() - 5.0, submenu_pos.y.min(rect.top())),  // Top of submenu or item
            egui::pos2(submenu_pos.x + 5.0, (submenu_pos.y + estimated_height).max(rect.bottom())),  // Bottom of submenu or item
        );
        
        let pointer_in_item = rect.contains(pointer_pos.unwrap_or(egui::Pos2::ZERO));
        let pointer_in_submenu = pointer_pos.map_or(false, |p| estimated_submenu_rect.contains(p));
        let pointer_in_bridge = pointer_pos.map_or(false, |p| bridge_rect.contains(p));
        
        // If hovering over this item, update the hierarchy at this depth level
        if response.hovered() {
            SUBMENU_HIERARCHY.with(|hierarchy| {
                let mut h = hierarchy.borrow_mut();
                // Ensure vector is large enough
                while h.len() <= depth {
                    h.push(None);
                }
                h[depth] = Some(item.id);
                // Clear deeper levels when a new item is selected at this depth
                h.truncate(depth + 1);
            });
        }
        
        // Check if THIS item is active at its depth level
        let is_active = SUBMENU_HIERARCHY.with(|hierarchy| {
            let h = hierarchy.borrow();
            h.get(depth).copied().flatten() == Some(item.id)
        });
        
        // Show submenu ONLY if:
        // 1. This is the active item at this depth, AND
        // 2. Pointer is in interaction zone (hovered, bridge, or submenu)
        let should_show_submenu = is_active && (response.hovered() || pointer_in_bridge || pointer_in_submenu);
        
        // Show submenu only if pointer is in the interaction zone
        if should_show_submenu {
            egui::Area::new(egui::Id::new(format!("submenu_{}", item.id)))
                .order(egui::Order::Foreground)
                .fixed_pos(submenu_pos)
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style())
                        .inner_margin(4.0)
                        .corner_radius(MENU_ROUNDING)
                        .show(ui, |ui| {
                            ui.set_min_width(SUBMENU_MIN_WIDTH);
                            ui.set_max_width(MENU_MAX_WIDTH);
                            ui.spacing_mut().item_spacing = egui::vec2(0.0, 1.0);
                            for sub in &item.sub_items {
                                render_single_item(ui, sub, action, depth + 1);  // Pass depth + 1 for nested submenu
                            }
                        });
                });
        }
    }
}

/// Render "Show more options" overflow submenu
fn render_overflow_submenu(
    ui: &mut egui::Ui,
    items: &[&ContextMenuItem],
    action: &mut Option<i32>,
) {
    let overflow_item = ContextMenuItem::new(-100, "Mostrar mais opções")
        .with_subitems(items.iter().map(|i| (*i).clone()).collect());

    render_single_item(ui, &overflow_item, action, 0);  // Overflow is top-level
}
