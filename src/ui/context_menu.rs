//! Context menu rendering (Files-style 1:1 clone)
//! Follows .cursorrules: single responsibility, < 300 lines

use eframe::egui::{self, Sense};
use std::cell::RefCell;

pub use crate::application::context_menu::{ContextMenuState, ContextMenuItem};
use crate::ui::svg_icons::SvgIconManager;

// Track submenu hierarchy: each depth level stores which item is active at that level
// Example: hovering "7-Zip" -> [Some(7zip_id)]
// Hovering item inside 7-Zip submenu -> [Some(7zip_id), Some(sub_item_id)]
thread_local! {
    static SUBMENU_HIERARCHY: RefCell<Vec<Option<i32>>> = RefCell::new(Vec::new());
}

/// Operations that can be performed from context menu
pub trait ContextMenuOperations {
    fn create_new_folder(&mut self);
    fn command_copy(&mut self, idx: Option<usize>);
    fn command_cut(&mut self, idx: Option<usize>);
    fn command_paste(&mut self, idx: Option<usize>);
    fn rename_item(&mut self, idx: usize);
    fn delete_with_shell(&mut self, idx: Option<usize>);
}

/// Menu styling constants (matching Files app - compact)
const HEADER_ICON_SIZE: f32 = 16.0;  // Display size
const HEADER_ICON_RENDER_SIZE: u32 = 32;  // Render at 2x for HiDPI quality
const HEADER_BUTTON_SIZE: f32 = 28.0;
const HEADER_SPACING: f32 = 4.0;
const ITEM_HEIGHT: f32 = 24.0;
const ITEM_ICON_SIZE: f32 = 18.0;
const MENU_ROUNDING: f32 = 6.0;
const MENU_MIN_WIDTH: f32 = 180.0;
const MENU_MAX_WIDTH: f32 = 400.0;
const SUBMENU_MIN_WIDTH: f32 = 220.0;
const SUBMENU_X_OFFSET: f32 = 6.0;
const SHORTCUT_COLOR: egui::Color32 = egui::Color32::from_gray(128);

/// SVG icon names for header bar (matching main toolbar style)
const SVG_ICON_CUT: &str = "cut";
const SVG_ICON_COPY: &str = "copy";
const SVG_ICON_PASTE: &str = "paste";
const SVG_ICON_RENAME: &str = "rename";
const SVG_ICON_DELETE: &str = "delete";
const SVG_ICON_PROPERTIES: &str = "properties";

/// Renders the Files-style context menu
pub fn render_context_menu(
    ctx: &egui::Context,
    menu_state: &mut ContextMenuState,
    svg_icon_manager: &mut SvgIconManager,
) -> bool {
    if !menu_state.is_open {
        // CRITICAL: Clear hierarchy when menu is not open
        SUBMENU_HIERARCHY.with(|hierarchy| {
            hierarchy.borrow_mut().clear();
        });
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
                        render_header_bar(ui, &primary_items, &mut action_executed, svg_icon_manager);
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
        // CRITICAL: Clear submenu hierarchy when menu closes
        SUBMENU_HIERARCHY.with(|hierarchy| {
            hierarchy.borrow_mut().clear();
        });
        return true;
    }

    false
}

/// Render the header bar with primary action icons
fn render_header_bar(
    ui: &mut egui::Ui,
    items: &[&ContextMenuItem],
    action: &mut Option<i32>,
    svg_icon_manager: &mut SvgIconManager,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(HEADER_SPACING, 0.0);
        
        // Determine icon color based on theme
        let icon_color = if ui.visuals().dark_mode {
            [220, 220, 220, 255]
        } else {
            [60, 60, 60, 255]
        };
        let disabled_color = [128, 128, 128, 180];
        
        for item in items {
            let btn_size = egui::vec2(HEADER_BUTTON_SIZE, HEADER_BUTTON_SIZE);
            
            // Get SVG icon name based on command_string
            let svg_icon_name = match item.command_string.as_deref() {
                Some("cut") => SVG_ICON_CUT,
                Some("copy") => SVG_ICON_COPY,
                Some("paste") => SVG_ICON_PASTE,
                Some("rename") => SVG_ICON_RENAME,
                Some("delete") => SVG_ICON_DELETE,
                Some("properties") => SVG_ICON_PROPERTIES,
                _ => "info", // Fallback icon
            };
            
            // Choose color based on enabled state
            let color = if item.is_enabled { icon_color } else { disabled_color };
            
            // Try to load SVG icon at 2x resolution for HiDPI quality, fallback to text button
            let response = if let Some(texture) = svg_icon_manager.get_icon(
                ui.ctx(),
                svg_icon_name,
                HEADER_ICON_RENDER_SIZE,  // Render at higher resolution
                color,
            ) {
                let img = egui::Image::from_texture(egui::load::SizedTexture::new(
                    texture.id(),
                    egui::vec2(HEADER_ICON_SIZE, HEADER_ICON_SIZE),
                ));
                ui.add_sized(btn_size, egui::ImageButton::new(img).frame(false))
            } else if let Some(icon) = &item.icon {
                // Use texture icon if available (from shell)
                let img = egui::Image::from_texture(egui::load::SizedTexture::new(
                    icon.id(),
                    egui::vec2(HEADER_ICON_SIZE, HEADER_ICON_SIZE),
                ));
                ui.add_sized(btn_size, egui::ImageButton::new(img))
            } else {
                // Last resort: use first letter as fallback
                let fallback = item.text.chars().next().unwrap_or('?').to_string();
                let btn = egui::Button::new(egui::RichText::new(fallback).size(12.0));
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

    // Items have submenu if they have sub_items OR a pending submenu to load
    let has_submenu = !item.sub_items.is_empty() || item.has_pending_submenu;
    
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
    let display_text = if item.text.len() > 45 {
        format!("{}…", &item.text[..43])
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

    // Handle submenu on hover - stable approach with expanded hit area
    if has_submenu {
        let pointer_pos = ui.ctx().pointer_latest_pos();
        
        // Calculate submenu position: RIGHT by default, LEFT only if insufficient space
        let screen_rect = ui.ctx().screen_rect();
        let menu_width = SUBMENU_MIN_WIDTH; // Expected submenu width

        let space_on_right = screen_rect.right() - rect.right();
        let needs_flip = space_on_right < (menu_width + SUBMENU_X_OFFSET + 20.0); // 20px margin

        // Open to the right by default, flip to left only if not enough space
        let open_left = needs_flip;

        let submenu_pos = if open_left {
            egui::pos2(rect.left() - menu_width - SUBMENU_X_OFFSET, rect.top())
        } else {
            egui::pos2(rect.right() + SUBMENU_X_OFFSET, rect.top())
        };
        
        // Create an EXPANDED rect that includes the submenu direction area
        // This prevents flickering when mouse is between parent and submenu
        let expanded_rect = if open_left {
            // Submenu is to the left - expand rect leftward
            egui::Rect::from_min_max(
                egui::pos2(rect.left() - SUBMENU_X_OFFSET - 10.0, rect.min.y),
                rect.max
            )
        } else {
            // Submenu is to the right - expand rect rightward
            egui::Rect::from_min_max(
                rect.min,
                egui::pos2(rect.right() + SUBMENU_X_OFFSET + 10.0, rect.max.y)
            )
        };
        
        // Check if pointer is in the expanded area (item + gap to submenu)
        let pointer_in_expanded_area = pointer_pos.map_or(false, |p| expanded_rect.contains(p));
        
        // CRITICAL: Check if there are DEEPER levels active (nested submenus)
        // If so, don't interfere with them by activating this item
        let has_deeper_active = SUBMENU_HIERARCHY.with(|hierarchy| {
            let h = hierarchy.borrow();
            // Check if any level deeper than current depth has an active item
            for i in (depth + 1)..h.len() {
                if h.get(i).copied().flatten().is_some() {
                    return true;
                }
            }
            false
        });

        // Track submenu rect so we can keep it open when pointer is over it
        let mut submenu_rect: Option<egui::Rect> = None;
        
        // Check if THIS item is active at its depth level FIRST (before updating hierarchy)
        let is_currently_active = SUBMENU_HIERARCHY.with(|hierarchy| {
            let h = hierarchy.borrow();
            h.get(depth).copied().flatten() == Some(item.id)
        });
        
        // If hovering over expanded area AND no deeper submenus are active, update hierarchy
        // CRITICAL: Only check if there's a SIBLING active at the SAME depth (not any level)
        // This allows nested submenus to open while preventing parent menu interference
        let sibling_submenu_active = SUBMENU_HIERARCHY.with(|hierarchy| {
            let h = hierarchy.borrow();
            // Check if there's an active item at THIS depth that is NOT this item
            if let Some(active_id) = h.get(depth).copied().flatten() {
                active_id != item.id
            } else {
                false
            }
        });
        
        // Only update hierarchy if: in expanded area, no deeper active, AND (no sibling active OR this is already active)
        let should_activate = pointer_in_expanded_area && !has_deeper_active && (!sibling_submenu_active || is_currently_active);
        
        if should_activate {
            SUBMENU_HIERARCHY.with(|hierarchy| {
                let mut h = hierarchy.borrow_mut();
                
                // Check if we're switching to a different item at this depth
                let is_different_item = h.get(depth).copied().flatten() != Some(item.id);
                
                while h.len() <= depth {
                    h.push(None);
                }
                h[depth] = Some(item.id);
                
                // CRITICAL: Clear deeper levels when switching items at this depth
                // This prevents submenu interference
                if is_different_item {
                    h.truncate(depth + 1);
                }
            });
        }
        
        // Check if THIS item is active at its depth level
        let is_active = SUBMENU_HIERARCHY.with(|hierarchy| {
            let h = hierarchy.borrow();
            h.get(depth).copied().flatten() == Some(item.id)
        });
        
        // Show submenu if this is the active item at this depth
        if is_active {
            let area_response = egui::Area::new(egui::Id::new(format!("submenu_{}", item.id)))
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
                                render_single_item(ui, sub, action, depth + 1);
                            }
                        });
                });

            // Store submenu rect for hover detection outside the main menu rect
            submenu_rect = Some(area_response.response.rect);
            
            // If pointer is inside the submenu area, keep the hierarchy active
            if let Some(pos) = pointer_pos {
                if area_response.response.rect.contains(pos) {
                    // Pointer is inside submenu - ensure this item stays active
                    SUBMENU_HIERARCHY.with(|hierarchy| {
                        let mut h = hierarchy.borrow_mut();
                        if h.get(depth).copied().flatten() != Some(item.id) {
                            while h.len() <= depth {
                                h.push(None);
                            }
                            h[depth] = Some(item.id);
                        }
                    });
                }
            }
        }

        // If pointer is neither on the parent nor on the submenu, and there's no deeper submenu active, clear this level
        let pointer_in_submenu = pointer_pos.map_or(false, |p| {
            submenu_rect.map_or(false, |r| r.contains(p))
        });

        if !pointer_in_expanded_area && !pointer_in_submenu && !has_deeper_active {
            SUBMENU_HIERARCHY.with(|hierarchy| {
                let mut h = hierarchy.borrow_mut();
                if h.get(depth).copied().flatten() == Some(item.id) {
                    h[depth] = None;
                    h.truncate(depth + 1);
                }
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
