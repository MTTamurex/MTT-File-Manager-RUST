//! Context menu rendering (Files-style 1:1 clone)
//! Follows .cursorrules: single responsibility, < 300 lines

use eframe::egui::{self, Sense};

use crate::application::context_menu::{ContextMenuState, ContextMenuItem};

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
const HEADER_ICON_SIZE: f32 = 16.0;
const HEADER_BUTTON_SIZE: f32 = 28.0;
const HEADER_SPACING: f32 = 2.0;
const ITEM_HEIGHT: f32 = 24.0;
const ITEM_ICON_SIZE: f32 = 16.0;
const MENU_WIDTH: f32 = 200.0;
const MENU_ROUNDING: f32 = 8.0;
const SHORTCUT_COLOR: egui::Color32 = egui::Color32::from_gray(128);

/// Unicode icons for header bar (matching Files/Windows 11 style)
const ICON_CUT: &str = "✂";      // Scissors
const ICON_COPY: &str = "📋";    // Clipboard  
const ICON_PASTE: &str = "📄";   // Document
const ICON_RENAME: &str = "✏";  // Pencil
const ICON_DELETE: &str = "🗑"; // Trash
const ICON_PROPERTIES: &str = "⚙"; // Gear

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
                .shadow(egui::epaint::Shadow::NONE)
                .show(ui, |ui| {
                    ui.set_min_width(MENU_WIDTH);
                    ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);

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

/// Render the header bar with primary action icons (Cut, Copy, Paste, Delete, Rename, Properties)
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
                let btn = egui::Button::new(egui::RichText::new(icon_str).size(14.0));
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
    for item in items {
        render_single_item(ui, item, action);
    }
}

/// Render a single menu item
fn render_single_item(
    ui: &mut egui::Ui,
    item: &ContextMenuItem,
    action: &mut Option<i32>,
) {
    if item.is_separator {
        ui.separator();
        return;
    }

    let has_submenu = !item.sub_items.is_empty();
    let row_rect = ui.available_rect_before_wrap();
    let desired_size = egui::vec2(ui.available_width(), ITEM_HEIGHT);
    
    // Allocate space for the row
    let (rect, response) = ui.allocate_at_least(desired_size, Sense::click());
    
    // Hover highlight
    if response.hovered() {
        ui.painter().rect_filled(
            rect,
            4.0,
            ui.visuals().widgets.hovered.bg_fill,
        );
    }

    // Icon (16x16)
    let icon_rect = egui::Rect::from_min_size(
        rect.min + egui::vec2(8.0, (ITEM_HEIGHT - ITEM_ICON_SIZE) / 2.0),
        egui::vec2(ITEM_ICON_SIZE, ITEM_ICON_SIZE),
    );
    
    if let Some(icon) = &item.icon {
        let img = egui::Image::from_texture(egui::load::SizedTexture::new(
            icon.id(),
            icon_rect.size(),
        ));
        img.paint_at(ui, icon_rect);
    }

    // Text
    let text_pos = rect.min + egui::vec2(8.0 + ITEM_ICON_SIZE + 8.0, (ITEM_HEIGHT - 14.0) / 2.0);
    let text_color = if item.is_enabled {
        ui.visuals().text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    
    ui.painter().text(
        text_pos,
        egui::Align2::LEFT_TOP,
        &item.text,
        egui::FontId::proportional(13.0),
        text_color,
    );

    // Keyboard shortcut or submenu arrow (right side)
    let right_margin = 12.0;
    if has_submenu {
        let arrow_pos = egui::pos2(rect.right() - right_margin, rect.center().y);
        ui.painter().text(
            arrow_pos,
            egui::Align2::RIGHT_CENTER,
            "›",
            egui::FontId::proportional(16.0),
            text_color,
        );
    } else if let Some(shortcut) = &item.keyboard_shortcut {
        let shortcut_pos = egui::pos2(rect.right() - right_margin, rect.center().y);
        ui.painter().text(
            shortcut_pos,
            egui::Align2::RIGHT_CENTER,
            shortcut,
            egui::FontId::proportional(11.0),
            SHORTCUT_COLOR,
        );
    }

    // Handle click
    if response.clicked() && item.is_enabled && !has_submenu {
        *action = Some(item.id);
    }

    // Handle submenu on hover
    if has_submenu && response.hovered() {
        // Show submenu to the right
        let submenu_pos = egui::pos2(rect.right() + 2.0, rect.top());
        egui::Area::new(egui::Id::new(format!("submenu_{}", item.id)))
            .fixed_pos(submenu_pos)
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style())
                    .inner_margin(8.0)
                    .corner_radius(MENU_ROUNDING)
                    .show(ui, |ui| {
                        ui.set_min_width(200.0);
                        for sub in &item.sub_items {
                            render_single_item(ui, sub, action);
                        }
                    });
            });
    }
}

/// Render "Show more options" overflow submenu
fn render_overflow_submenu(
    ui: &mut egui::Ui,
    items: &[&ContextMenuItem],
    action: &mut Option<i32>,
) {
    let overflow_item = ContextMenuItem::new(-100, "Show more options")
        .with_subitems(items.iter().map(|i| (*i).clone()).collect());
    
    render_single_item(ui, &overflow_item, action);
}
