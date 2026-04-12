use crate::app::state::sidebar_tree_state::{FolderNode, SidebarTreeState};
use eframe::egui::{self, Color32, Pos2, Rect, Sense};
use std::path::Path;

/// Actions emitted by the folder tree widget.
pub enum SidebarTreeAction {
    /// User clicked a folder name — navigate the central panel there.
    NavigateTo(String),
    /// User clicked the expand/collapse arrow on a node.
    ToggleExpand(std::path::PathBuf),
    /// Items were dropped onto a tree node folder.
    DropItemsTo(String),
}

/// Shared context references needed for tree rendering.
pub struct SidebarTreeContext<'a> {
    pub tree_state: &'a SidebarTreeState,
    pub current_path: &'a str,
    pub icon_loader: &'a mut crate::ui::icon_loader::IconLoader,
    pub is_renaming: bool,
    /// Whether an item drag is currently in progress (from the main content area).
    pub is_item_dragging: bool,
}

const ROW_HEIGHT: f32 = 24.0;
const INDENT_PX: f32 = 16.0;
const ARROW_WIDTH: f32 = 16.0;
const ICON_SIZE: f32 = 16.0;
const BASE_INDENT: f32 = 20.0; // Extra indent for tree nodes vs drive row

/// Render the folder tree for a single drive root.
/// Returns the first action triggered this frame (if any).
pub fn render_drive_tree(
    ui: &mut egui::Ui,
    drive_path: &str,
    ctx: &mut SidebarTreeContext,
) -> Option<SidebarTreeAction> {
    let root = Path::new(drive_path);
    let mut action: Option<SidebarTreeAction> = None;

    let tree_state = ctx.tree_state;
    let current_path = ctx.current_path;
    let is_renaming = ctx.is_renaming;

    // If the drive root is expanded, render its children
    if tree_state.is_expanded(root) {
        if tree_state.is_loading(root) && tree_state.get_children(root).is_none() {
            // Show loading indicator
            render_loading_row(ui, 1);
        } else if let Some(children) = tree_state.get_children(root) {
            for node in children {
                let is_item_dragging = ctx.is_item_dragging;
                let mut node_ctx = SidebarTreeContext {
                    tree_state,
                    current_path,
                    icon_loader: &mut *ctx.icon_loader,
                    is_renaming,
                    is_item_dragging,
                };
                render_tree_node(ui, node, 1, &mut node_ctx, &mut action);
            }
        }
    }

    action
}

/// Render a single tree node and recurse into its expanded children.
fn render_tree_node(
    ui: &mut egui::Ui,
    node: &FolderNode,
    depth: usize,
    ctx: &mut SidebarTreeContext,
    action: &mut Option<SidebarTreeAction>,
) {
    let indent = BASE_INDENT + (depth as f32) * INDENT_PX;
    let is_expanded = ctx.tree_state.is_expanded(&node.path);
    let is_loading = ctx.tree_state.is_loading(&node.path);
    let has_children = node.has_subfolders.unwrap_or(true); // optimistic: show arrow until proven empty
    let is_selected = ctx.current_path.eq_ignore_ascii_case(
        &node.path.to_string_lossy(),
    );

    // Allocate row — use max of available width and content width so deep nodes
    // push the ScrollArea's content wider, enabling horizontal scroll.
    // Approximate text width with per-char estimate (avoids font shaping every frame).
    let approx_text_width = node.name.len() as f32 * 7.0;
    let content_min_width = indent + ARROW_WIDTH + ICON_SIZE + 4.0
        + approx_text_width
        + 8.0; // right padding
    let row_width = ui.available_width().max(content_min_width);
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(row_width, ROW_HEIGHT), Sense::click());

    if ui.is_rect_visible(rect) {
        let dark_mode = ui.visuals().dark_mode;
        let hidden_opacity = if node.is_hidden { 0.5 } else { 1.0 };

        // Detect if the pointer is hovering this row during an external item drag.
        // We check hover_pos manually because egui's response.hovered() won't fire
        // when the drag originated from a different widget.
        let drag_hover = ctx.is_item_dragging
            && ui.input(|inp| inp.pointer.hover_pos())
                .map(|p| rect.contains(p))
                .unwrap_or(false);

        // Row background
        if drag_hover {
            // Blue border to indicate valid drop target (matches content panel style)
            ui.painter().rect_stroke(
                rect,
                0.0,
                egui::Stroke::new(2.0, Color32::from_rgb(24, 122, 255)),
                egui::StrokeKind::Inside,
            );
        } else if is_selected {
            ui.painter()
                .rect_filled(rect, 0.0, crate::ui::theme::selection_color(dark_mode));
        } else if response.hovered() && !ctx.is_item_dragging {
            ui.painter()
                .rect_filled(rect, 0.0, crate::ui::theme::selection_hover_color(dark_mode));
        }

        let mut cursor_x = rect.min.x + indent;

        // ── Arrow (expand/collapse indicator) ──
        if has_children || is_loading {
            let arrow_rect = Rect::from_center_size(
                Pos2::new(cursor_x + ARROW_WIDTH / 2.0, rect.center().y),
                egui::vec2(ARROW_WIDTH, ROW_HEIGHT),
            );

            let arrow_text = if is_loading {
                "…"
            } else if is_expanded {
                "▾"
            } else {
                "▸"
            };

            let arrow_color = if response.hovered() {
                ui.visuals().text_color()
            } else {
                Color32::from_gray(140)
            }.gamma_multiply(hidden_opacity);

            ui.painter().text(
                arrow_rect.center(),
                egui::Align2::CENTER_CENTER,
                arrow_text,
                egui::FontId::proportional(11.0),
                arrow_color,
            );
        }
        cursor_x += ARROW_WIDTH;

        // ── Folder Icon ──
        let folder_icon = ctx.icon_loader.get_or_load_folder_path_icon(
            ui.ctx(),
            &node.path.to_string_lossy(),
        );
        if let Some(icon) = folder_icon {
            let icon_rect = Rect::from_center_size(
                Pos2::new(cursor_x + ICON_SIZE / 2.0, rect.center().y),
                egui::vec2(ICON_SIZE, ICON_SIZE),
            );
            ui.painter().image(
                icon.id(),
                icon_rect,
                Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                Color32::WHITE.gamma_multiply(hidden_opacity),
            );
            cursor_x += ICON_SIZE + 4.0;
        } else {
            cursor_x += ICON_SIZE + 4.0;
        }

        // ── Folder Name ──
        let text_color = if is_selected {
            crate::ui::theme::selection_text_color(dark_mode)
        } else {
            ui.visuals().text_color()
        }.gamma_multiply(hidden_opacity);

        ui.painter().text(
            Pos2::new(cursor_x, rect.center().y),
            egui::Align2::LEFT_CENTER,
            &node.name,
            egui::FontId::proportional(11.0),
            text_color,
        );
    }

    // ── Handle clicks ──
    if (response.double_clicked() || response.clicked())
        && action.is_none()
        && !ctx.is_renaming
        && !ctx.is_item_dragging
    {
        // Determine if click was on the arrow area
        let click_pos = ui.input(|inp| inp.pointer.interact_pos());
        let arrow_end_x = rect.min.x + indent + ARROW_WIDTH;

        let clicked_arrow = click_pos
            .map(|p| p.x < arrow_end_x && p.x >= rect.min.x + indent)
            .unwrap_or(false);
        let can_toggle = has_children || is_loading;

        if response.double_clicked() {
            if can_toggle {
                *action = Some(SidebarTreeAction::ToggleExpand(node.path.clone()));
            } else {
                *action = Some(SidebarTreeAction::NavigateTo(
                    node.path.to_string_lossy().into_owned(),
                ));
            }
        } else if clicked_arrow && can_toggle {
            *action = Some(SidebarTreeAction::ToggleExpand(node.path.clone()));
        } else {
            *action = Some(SidebarTreeAction::NavigateTo(
                node.path.to_string_lossy().into_owned(),
            ));
        }
    }

    // ── Handle drop from external item drag ──
    if ctx.is_item_dragging && action.is_none() {
        let pointer_over = ui.input(|inp| inp.pointer.hover_pos())
            .map(|p| rect.contains(p))
            .unwrap_or(false);
        let released = ui.input(|inp| inp.pointer.primary_released());

        if released && pointer_over {
            *action = Some(SidebarTreeAction::DropItemsTo(
                node.path.to_string_lossy().into_owned(),
            ));
        }
    }

    // ── Recurse into children (if expanded) ──
    if is_expanded && action.is_none() {
        if is_loading && ctx.tree_state.get_children(&node.path).is_none() {
            render_loading_row(ui, depth + 1);
        } else if let Some(children) = ctx.tree_state.get_children(&node.path) {
            let tree_state = ctx.tree_state;
            let current_path = ctx.current_path;
            let is_renaming = ctx.is_renaming;
            let is_item_dragging = ctx.is_item_dragging;
            for child in children {
                let mut child_ctx = SidebarTreeContext {
                    tree_state,
                    current_path,
                    icon_loader: &mut *ctx.icon_loader,
                    is_renaming,
                    is_item_dragging,
                };
                render_tree_node(ui, child, depth + 1, &mut child_ctx, action);
            }
        }
    }
}

/// Render a "loading..." placeholder row at the given depth.
fn render_loading_row(ui: &mut egui::Ui, depth: usize) {
    let indent = BASE_INDENT + (depth as f32) * INDENT_PX + ARROW_WIDTH;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_HEIGHT), Sense::hover());

    if ui.is_rect_visible(rect) {
        ui.painter().text(
            Pos2::new(rect.min.x + indent, rect.center().y),
            egui::Align2::LEFT_CENTER,
            "…",
            egui::FontId::proportional(11.0),
            Color32::from_gray(120),
        );
    }
}

