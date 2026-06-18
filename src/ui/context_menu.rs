//! Context menu rendering (Files-style 1:1 clone)
//! Follows .cursorrules: single responsibility, < 300 lines

use eframe::egui::{self, Sense};
use std::cell::RefCell;

pub use crate::application::context_menu::{ContextMenuItem, ContextMenuState};
use crate::ui::svg_icons::SvgIconManager;

// Track submenu hierarchy: each depth level stores which item is active at that level
// Example: hovering "7-Zip" -> [Some(7zip_id)]
// Hovering item inside 7-Zip submenu -> [Some(7zip_id), Some(sub_item_id)]
thread_local! {
    static SUBMENU_HIERARCHY: RefCell<Vec<Option<i32>>> = const { RefCell::new(Vec::new()) };
}

fn suppress_tooltips_id() -> egui::Id {
    egui::Id::new("suppress_tooltips_for_context_menu")
}

/// Sets the tooltip suppression flag so item tooltips are hidden while the menu is open.
pub fn set_tooltip_suppression(ctx: &egui::Context, suppress: bool) {
    ctx.data_mut(|d| d.insert_temp(suppress_tooltips_id(), suppress));
}

/// Returns true if tooltips should be suppressed (context menu is open).
pub fn should_suppress_tooltips(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp(suppress_tooltips_id()).unwrap_or(false))
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
const HEADER_ICON_SIZE: f32 = 20.0; // Display size
const HEADER_ICON_RENDER_SIZE: u32 = 40; // Render at 2x for HiDPI quality
const HEADER_BUTTON_WIDTH: f32 = 56.0;
const HEADER_BUTTON_HEIGHT: f32 = 48.0;
const HEADER_SPACING: f32 = 8.0;
const ITEM_HEIGHT: f32 = 28.0;
const ITEM_ICON_SIZE: f32 = 16.0;
const ICON_TEXT_GAP: f32 = 10.0;
const MENU_ROUNDING: f32 = 6.0;
const MENU_MIN_WIDTH: f32 = 180.0;
const MENU_MAX_WIDTH: f32 = 400.0;
const SUBMENU_MIN_WIDTH: f32 = 220.0;
const SUBMENU_X_OFFSET: f32 = 6.0;
const SHORTCUT_COLOR: egui::Color32 = egui::Color32::from_gray(128);
const HOVER_H_MARGIN: f32 = 2.0;

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
        set_tooltip_suppression(ctx, false);
        return false;
    }

    // Suppress item tooltips while the context menu is visible
    set_tooltip_suppression(ctx, true);

    let mut action_executed: Option<i32> = None;
    let mut pending_load_item: Option<i32> = None;
    let mut should_close = false;

    // M-5: Ensure partition indices are up-to-date (normally pre-computed at item assignment).
    if menu_state.partition_dirty {
        menu_state.partition_items();
    }

    // Use pre-partitioned indices — no per-frame filter().collect()
    let primary_items: Vec<&ContextMenuItem> = menu_state
        .primary_indices
        .iter()
        .map(|&i| &menu_state.items[i])
        .collect();
    let secondary_items: Vec<&ContextMenuItem> = menu_state
        .secondary_indices
        .iter()
        .map(|&i| &menu_state.items[i])
        .collect();
    let overflow_items: Vec<&ContextMenuItem> = menu_state
        .overflow_indices
        .iter()
        .map(|&i| &menu_state.items[i])
        .collect();

    // Calculate menu width to fit header bar tightly (Windows 11 style)
    let menu_width = if !primary_items.is_empty() {
        let header_width = primary_items.len() as f32 * HEADER_BUTTON_WIDTH
            + (primary_items.len().saturating_sub(1)) as f32 * HEADER_SPACING
            + 8.0; // inner_margin * 2
        header_width.max(MENU_MIN_WIDTH)
    } else {
        MENU_MIN_WIDTH
    };

    // SMART ALIGNMENT: If the menu would open over the player area, shift it to the left
    let mut menu_pos = menu_state.position;
    let expected_width = menu_width;

    if menu_pos.x + expected_width > menu_state.right_bound {
        // If it hits the right edge, move the menu to the left of the cursor
        menu_pos.x = (menu_state.right_bound - expected_width).max(0.0);
    }

    // VERTICAL CLAMPING: Prevent menu from extending below the screen
    let screen_rect = ctx.screen_rect();
    let separator_count = if !primary_items.is_empty() { 1 } else { 0 }
        + if !overflow_items.is_empty() { 1 } else { 0 };
    let expected_height = (HEADER_BUTTON_HEIGHT + 8.0) * (!primary_items.is_empty() as u32 as f32)
        + (secondary_items.len() as f32 * (ITEM_HEIGHT + 1.0))
        + (overflow_items.len().min(1) as f32 * (ITEM_HEIGHT + 1.0))
        + (separator_count as f32 * 6.0)
        + 8.0; // inner_margin * 2

    if menu_pos.y + expected_height > screen_rect.bottom() {
        menu_pos.y = (screen_rect.bottom() - expected_height).max(0.0);
    }

    // Render the menu popup
    let response = egui::Area::new(egui::Id::new("context_menu"))
        .fixed_pos(menu_pos)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style())
                .inner_margin(4.0)
                .corner_radius(MENU_ROUNDING)
                .show(ui, |ui| {
                    ui.set_min_width(menu_width); // Tight fit to header bar
                    ui.set_max_width(menu_width); // Prevent extra empty space
                    ui.spacing_mut().item_spacing = egui::vec2(0.0, 1.0);

                    // ========== HEADER BAR (Primary items as icons) ==========
                    if !primary_items.is_empty() {
                        render_header_bar(
                            ui,
                            &primary_items,
                            &mut action_executed,
                            svg_icon_manager,
                        );
                        ui.separator();
                    }

                    // ========== SECONDARY ITEMS (Regular menu items) ==========
                    render_menu_items(
                        ui,
                        &secondary_items,
                        &mut action_executed,
                        &mut pending_load_item,
                        menu_state.right_bound,
                        svg_icon_manager,
                    );

                    // ========== OVERFLOW ("Show more options") ==========
                    if !overflow_items.is_empty() {
                        render_overflow_submenu(
                            ui,
                            &overflow_items,
                            &mut action_executed,
                            &mut pending_load_item,
                            menu_state.right_bound,
                            svg_icon_manager,
                        );
                    }
                });
        });

    // Handle action execution
    if let Some(id) = action_executed {
        menu_state.selected_command_id = Some(id);
        should_close = true;
    }

    if let Some(id) = pending_load_item {
        menu_state.pending_load_item = Some(id);
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

    // Close on Enter (item already opened by keyboard handler in slots)
    if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
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

/// Render the header bar with primary action icons + labels (Windows 11 style)
fn render_header_bar(
    ui: &mut egui::Ui,
    items: &[&ContextMenuItem],
    action: &mut Option<i32>,
    svg_icon_manager: &mut SvgIconManager,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(HEADER_SPACING, 0.0);

        // Determine colors based on theme
        let icon_color = if ui.visuals().dark_mode {
            [220, 220, 220, 255]
        } else {
            [60, 60, 60, 255]
        };
        let disabled_color = [128, 128, 128, 180];
        let text_color = if ui.visuals().dark_mode {
            egui::Color32::from_gray(220)
        } else {
            egui::Color32::from_gray(60)
        };
        let disabled_text_color = egui::Color32::from_gray(128);

        for item in items {
            let btn_size = egui::vec2(HEADER_BUTTON_WIDTH, HEADER_BUTTON_HEIGHT);
            let (rect, response) = ui.allocate_exact_size(btn_size, Sense::click());

            // Hover highlight
            if response.hovered() {
                let hover_bg = if ui.visuals().dark_mode {
                    egui::Color32::from_white_alpha(20)
                } else {
                    egui::Color32::from_black_alpha(20)
                };
                ui.painter().rect_filled(rect, 4.0, hover_bg);
            }

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
            let color = if item.is_enabled {
                icon_color
            } else {
                disabled_color
            };
            let label_color = if item.is_enabled {
                text_color
            } else {
                disabled_text_color
            };

            // Draw icon centered in top portion
            let icon_y = rect.min.y + 6.0 + HEADER_ICON_SIZE / 2.0;
            let icon_rect = egui::Rect::from_center_size(
                egui::pos2(rect.center().x, icon_y),
                egui::vec2(HEADER_ICON_SIZE, HEADER_ICON_SIZE),
            );

            if let Some(texture) =
                svg_icon_manager.get_icon(ui.ctx(), svg_icon_name, HEADER_ICON_RENDER_SIZE, color)
            {
                ui.painter().image(
                    texture.id(),
                    icon_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            } else if let Some(icon) = &item.icon {
                let img = egui::Image::from_texture(egui::load::SizedTexture::new(
                    icon.id(),
                    icon_rect.size(),
                ));
                img.paint_at(ui, icon_rect);
            } else {
                let fallback = item.text.chars().next().unwrap_or('?').to_string();
                ui.painter().text(
                    icon_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    fallback,
                    egui::FontId::proportional(14.0),
                    label_color,
                );
            }

            // Draw label centered at bottom
            let label_y = rect.max.y - 8.0;
            ui.painter().text(
                egui::pos2(rect.center().x, label_y),
                egui::Align2::CENTER_CENTER,
                &item.text,
                egui::FontId::proportional(10.0),
                label_color,
            );

            // Tooltip with shortcut
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
    lazy_load: &mut Option<i32>,
    right_bound: f32,
    svg_icon_manager: &mut SvgIconManager,
) {
    let mut last_was_separator = true; // collapse leading separators

    for item in items {
        if item.is_separator {
            if last_was_separator {
                continue; // skip duplicate/leading separators
            }
            render_single_item(
                ui,
                item,
                action,
                0,
                lazy_load,
                right_bound,
                svg_icon_manager,
            );
            last_was_separator = true;
        } else {
            render_single_item(
                ui,
                item,
                action,
                0,
                lazy_load,
                right_bound,
                svg_icon_manager,
            );
            last_was_separator = false;
        }
    }
}

/// Render a single menu item using egui's natural layout
fn render_single_item(
    ui: &mut egui::Ui,
    item: &ContextMenuItem,
    action: &mut Option<i32>,
    depth: usize,
    lazy_load: &mut Option<i32>,
    right_bound: f32,
    svg_icon_manager: &mut SvgIconManager,
) {
    if item.is_separator {
        // Custom styled separator with horizontal padding
        let sep_rect = ui.available_rect_before_wrap();
        let y = sep_rect.min.y + 4.0;
        let sep_color = if ui.visuals().dark_mode {
            egui::Color32::from_gray(60)
        } else {
            egui::Color32::from_gray(220)
        };
        ui.painter().hline(
            sep_rect.min.x + 10.0..=sep_rect.max.x - 10.0,
            y,
            egui::Stroke::new(1.0, sep_color),
        );
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 9.0), Sense::hover());
        return;
    }

    let item_text_lower = item.text.to_lowercase();
    let is_open_with_item =
        item_text_lower.contains("open with") || item_text_lower.contains("abrir com");
    let is_blocked_waiting_for_submenu =
        is_open_with_item && item.has_pending_submenu && item.sub_items.is_empty();
    let has_submenu =
        !item.sub_items.is_empty() || (item.has_pending_submenu && !is_blocked_waiting_for_submenu);

    // Build the label with icon + text + shortcut/arrow
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ITEM_HEIGHT),
        Sense::click(),
    );

    // Text color — loading placeholder uses a dimmed style
    let text_color = if item.is_loading_placeholder || is_blocked_waiting_for_submenu {
        // Dimmed "Loading…" style
        if ui.visuals().dark_mode {
            egui::Color32::from_gray(100)
        } else {
            egui::Color32::from_gray(160)
        }
    } else if item.is_enabled {
        ui.visuals().text_color()
    } else {
        ui.visuals().weak_text_color()
    };

    // Refined hover highlight (Windows 11 style soft blue with margin) — skip for loading placeholder
    if response.hovered() && !item.is_loading_placeholder && !is_blocked_waiting_for_submenu {
        let hover_color = if ui.visuals().dark_mode {
            egui::Color32::from_rgb(43, 84, 127)
        } else {
            egui::Color32::from_rgb(230, 243, 255)
        };
        let hover_rect = rect.shrink2(egui::vec2(HOVER_H_MARGIN, 0.0));
        ui.painter().rect_filled(hover_rect, 4.0, hover_color);
    }

    // Icon (16x16)
    let icon_rect = egui::Rect::from_min_size(
        egui::pos2(rect.min.x + 10.0, rect.center().y - ITEM_ICON_SIZE / 2.0),
        egui::vec2(ITEM_ICON_SIZE, ITEM_ICON_SIZE),
    );

    // Try SVG icon first, then shell texture fallback — skip for loading placeholder
    let mut icon_drawn = false;
    if !item.is_loading_placeholder {
        if let Some(svg_name) = &item.svg_icon_name {
            let icon_color_rgba = if item.is_enabled && !is_blocked_waiting_for_submenu {
                if ui.visuals().dark_mode {
                    [220, 220, 220, 255]
                } else {
                    [60, 60, 60, 255]
                }
            } else {
                [128, 128, 128, 180]
            };
            if let Some(texture) = svg_icon_manager.get_icon(
                ui.ctx(),
                svg_name,
                ITEM_ICON_SIZE as u32,
                icon_color_rgba,
            ) {
                ui.painter().image(
                    texture.id(),
                    icon_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
                icon_drawn = true;
            }
        }
        if !icon_drawn {
            if let Some(icon) = &item.icon {
                let img = egui::Image::from_texture(egui::load::SizedTexture::new(
                    icon.id(),
                    icon_rect.size(),
                ));
                img.paint_at(ui, icon_rect);
            }
        }
    }

    // Text with ellipsis truncation to prevent overflow
    let text_x = icon_rect.right() + ICON_TEXT_GAP;

    // Truncate very long names (like drive paths in "Send to") with ellipsis
    // Use char_indices to find proper UTF-8 boundaries (avoids panic on multi-byte chars)
    let display_text = if item.text.chars().count() > 45 {
        let truncate_char_count = 43;
        let byte_idx = item
            .text
            .char_indices()
            .nth(truncate_char_count)
            .map(|(idx, _)| idx)
            .unwrap_or(item.text.len());
        format!("{}…", &item.text[..byte_idx])
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

    // Handle click — loading placeholder is not interactive
    if response.clicked()
        && item.is_enabled
        && !has_submenu
        && !item.is_loading_placeholder
        && !is_blocked_waiting_for_submenu
    {
        *action = Some(item.id);
    }

    // Handle submenu on hover - stable approach with expanded hit area
    if has_submenu {
        let pointer_pos = ui.ctx().pointer_latest_pos();

        // Calculate submenu position: RIGHT by default, LEFT only if insufficient space
        let screen_rect = ui.ctx().screen_rect();
        let menu_width = SUBMENU_MIN_WIDTH; // Expected submenu width

        // SMART ALIGNMENT: Uses the real boundary (accounting for the video player)
        let effective_right = screen_rect.right().min(right_bound);
        let space_on_right = effective_right - rect.right();
        let needs_flip = space_on_right < (menu_width + SUBMENU_X_OFFSET + 20.0); // 20px margin

        // Open to the right by default, flip to left only if not enough space
        let open_left = needs_flip;

        let submenu_pos = if open_left {
            egui::pos2(rect.left() - menu_width - SUBMENU_X_OFFSET, rect.top())
        } else {
            egui::pos2(rect.right() + SUBMENU_X_OFFSET, rect.top())
        };

        // VERTICAL CLAMPING: Prevent submenu from extending below the screen
        let submenu_height = item.sub_items.len() as f32 * (ITEM_HEIGHT + 1.0) + 8.0;
        let submenu_pos = if submenu_pos.y + submenu_height > screen_rect.bottom() {
            egui::pos2(
                submenu_pos.x,
                (screen_rect.bottom() - submenu_height).max(0.0),
            )
        } else {
            submenu_pos
        };

        // Create an EXPANDED rect that includes the submenu direction area
        // This prevents flickering when mouse is between parent and submenu
        let expanded_rect = if open_left {
            // Submenu is to the left - expand rect leftward
            egui::Rect::from_min_max(
                egui::pos2(rect.left() - SUBMENU_X_OFFSET - 10.0, rect.min.y),
                rect.max,
            )
        } else {
            // Submenu is to the right - expand rect rightward
            egui::Rect::from_min_max(
                rect.min,
                egui::pos2(rect.right() + SUBMENU_X_OFFSET + 10.0, rect.max.y),
            )
        };

        // Check if pointer is in the expanded area (item + gap to submenu)
        let pointer_in_expanded_area = pointer_pos.is_some_and(|p| expanded_rect.contains(p));

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
        let should_activate = pointer_in_expanded_area
            && !has_deeper_active
            && (!sibling_submenu_active || is_currently_active);

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
            // Signal lazy load if submenu is active but empty
            if item.has_pending_submenu && item.sub_items.is_empty() {
                *lazy_load = Some(item.id);
            }

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
                                render_single_item(
                                    ui,
                                    sub,
                                    action,
                                    depth + 1,
                                    lazy_load,
                                    right_bound,
                                    svg_icon_manager,
                                );
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
        let pointer_in_submenu =
            pointer_pos.is_some_and(|p| submenu_rect.is_some_and(|r| r.contains(p)));

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
    lazy_load: &mut Option<i32>,
    right_bound: f32,
    svg_icon_manager: &mut SvgIconManager,
) {
    let overflow_item = ContextMenuItem::new(-100, rust_i18n::t!("context_menu.show_more"))
        .with_subitems(items.iter().map(|i| (*i).clone()).collect());

    render_single_item(
        ui,
        &overflow_item,
        action,
        0,
        lazy_load,
        right_bound,
        svg_icon_manager,
    );
}
