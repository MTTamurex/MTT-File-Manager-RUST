//! Compact icon + name renderer for a single Miller's Columns ancestor column.
//!
//! The focused (rightmost) column is rendered by the normal details list view;
//! this renderer is used only for the ancestor columns to its left. It supports
//! selection highlight, hover, and click / double-click / right-click, all
//! reported back to the bridge which applies path-based actions.

use std::hash::Hash;

use eframe::egui::{self, Color32, FontId, Rect, Sense, Ui};
use lru::LruCache;
use std::path::{Path, PathBuf};

use crate::domain::file_entry::FileEntry;
use crate::ui::icon_loader::IconLoader;
use crate::ui::theme;
use crate::ui::views::list_view::truncate_text_for_column;

use super::layout::COL_ROW_HEIGHT;

const ICON_SIZE: f32 = 16.0;
const LEFT_PAD: f32 = 8.0;
const CHEVRON_W: f32 = 14.0;

/// Action reported by a column when the user interacts with a row.
pub enum MillerColumnAction {
    Clicked(usize),
    DoubleClicked(usize),
    SecondaryClicked(usize, egui::Pos2),
    DragStarted(usize),
    EmptySecondaryClicked(egui::Pos2),
}

/// Borrowed context needed to render one ancestor column.
pub struct MillerColumnContext<'a> {
    pub items: &'a [FileEntry],
    pub directory: &'a Path,
    /// The child directory (from this column) that leads to the next column.
    pub selected_child: Option<&'a Path>,
    /// The currently previewed file/selection path (for highlight).
    pub selected_file: Option<&'a Path>,
    pub icon_loader: &'a mut IconLoader,
    pub folder_icon: Option<&'a egui::TextureHandle>,
    pub loading_icons: &'a crate::ui::cache::FxHashSet<PathBuf>,
    pub failed_icons: &'a LruCache<PathBuf, ()>,
    pub icon_requests: &'a mut Vec<PathBuf>,
    pub is_item_dragging: bool,
    pub drop_target: &'a mut Option<PathBuf>,
    pub is_loading: bool,
}

/// Render one ancestor column into the current (already column-sized) `ui`.
/// The caller allocates the column region (inside a horizontal `ScrollArea`),
/// so this renders directly without manual positioning or clip overrides.
pub fn render_miller_column(
    ui: &mut Ui,
    id_salt: impl Hash,
    ctx: &mut MillerColumnContext,
) -> Option<MillerColumnAction> {
    let dark = ui.visuals().dark_mode;
    let rect = ui.max_rect();
    let column_id = ui.make_persistent_id(id_salt);
    let background_response = ui.interact(rect, column_id.with("background"), Sense::click());

    if ctx.is_item_dragging
        && ui
            .input(|input| input.pointer.hover_pos())
            .is_some_and(|position| rect.contains(position))
    {
        *ctx.drop_target = Some(ctx.directory.to_path_buf());
    }

    // Right separator between columns.
    let sep_color = if dark {
        Color32::from_gray(60)
    } else {
        Color32::from_gray(210)
    };
    ui.painter().vline(
        rect.right(),
        rect.top()..=rect.bottom(),
        egui::Stroke::new(1.0, sep_color),
    );

    if ctx.items.is_empty() {
        if ctx.is_loading {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "…",
                FontId::proportional(13.0),
                Color32::from_gray(120),
            );
        }
        return background_secondary_action(&background_response, ui);
    }

    let mut action = None;
    egui::ScrollArea::vertical()
        .id_salt(column_id)
        .auto_shrink([false, false])
        .show_rows(ui, COL_ROW_HEIGHT, ctx.items.len(), |ui, row_range| {
            for index in row_range {
                let Some(item) = ctx.items.get(index) else {
                    continue;
                };
                if let Some(a) = render_row(ui, index, item, ctx, dark) {
                    action = Some(a);
                }
            }
        });

    action.or_else(|| background_secondary_action(&background_response, ui))
}

fn render_row(
    ui: &mut Ui,
    index: usize,
    item: &FileEntry,
    ctx: &mut MillerColumnContext,
    dark: bool,
) -> Option<MillerColumnAction> {
    let width = ui.available_width();
    let (row_rect, response) =
        ui.allocate_exact_size(egui::vec2(width, COL_ROW_HEIGHT), Sense::click_and_drag());

    let is_drop_candidate = ctx.is_item_dragging && response.contains_pointer() && item.is_dir;
    if is_drop_candidate {
        *ctx.drop_target = Some(item.path.clone());
    }

    if !ui.is_rect_visible(row_rect) {
        return interaction(index, &response, ui);
    }

    let is_selected = ctx.selected_child.is_some_and(|p| p == item.path.as_path())
        || ctx.selected_file.is_some_and(|p| p == item.path.as_path());
    let dim = if item.is_hidden { 0.5 } else { 1.0 };

    if is_selected {
        ui.painter()
            .rect_filled(row_rect, 0.0, theme::selection_color(dark));
    } else if response.hovered() {
        ui.painter()
            .rect_filled(row_rect, 0.0, theme::selection_hover_color(dark));
    }

    if is_drop_candidate {
        ui.painter().rect_stroke(
            row_rect.shrink(1.0),
            4.0,
            egui::Stroke::new(2.0, theme::COLOR_ACCENT),
            egui::StrokeKind::Inside,
        );
    }

    // Compact Miller columns always use Shell icons. The shared texture cache
    // also contains selection-preview thumbnails, which must not leak into rows.
    let icon = if item.is_dir {
        ctx.folder_icon.cloned()
    } else {
        ctx.icon_loader
            .get_or_load_icon(ui.ctx(), &item.path, false, false)
    };
    if !item.is_dir
        && icon.is_none()
        && !ctx.loading_icons.contains(&item.path)
        && ctx.failed_icons.peek(&item.path).is_none()
    {
        ctx.icon_requests.push(item.path.clone());
    }

    let icon_x = row_rect.left() + LEFT_PAD;
    if let Some(tex) = icon {
        let icon_rect = Rect::from_center_size(
            egui::pos2(icon_x + ICON_SIZE / 2.0, row_rect.center().y),
            egui::vec2(ICON_SIZE, ICON_SIZE),
        );
        ui.painter().image(
            tex.id(),
            icon_rect,
            Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE.gamma_multiply(dim),
        );
    }

    // Name text (truncated to fit).
    let text_x = icon_x + ICON_SIZE + 6.0;
    let text_color = if is_selected {
        theme::selection_text_color(dark)
    } else {
        ui.visuals().text_color()
    }
    .gamma_multiply(dim);
    let font = FontId::proportional(13.0);
    let max_text_w = (row_rect.right() - text_x - CHEVRON_W - 4.0).max(0.0);
    let display = truncate_text_for_column(&item.name, max_text_w, &font, ui);
    ui.painter().text(
        egui::pos2(text_x, row_rect.center().y),
        egui::Align2::LEFT_CENTER,
        display,
        font,
        text_color,
    );

    // Chevron for directories (drill-down affordance).
    if item.is_dir {
        ui.painter().text(
            egui::pos2(row_rect.right() - CHEVRON_W * 0.5, row_rect.center().y),
            egui::Align2::CENTER_CENTER,
            "›",
            FontId::proportional(15.0),
            Color32::from_gray(if dark { 150 } else { 120 }).gamma_multiply(dim),
        );
    }

    interaction(index, &response, ui)
}

fn interaction(index: usize, response: &egui::Response, ui: &Ui) -> Option<MillerColumnAction> {
    let (press_origin, pointer_position) =
        ui.input(|input| (input.pointer.press_origin(), input.pointer.hover_pos()));
    if crate::ui::views::common::should_start_item_drag(
        response.drag_started(),
        response.dragged(),
        response.is_pointer_button_down_on(),
        press_origin,
        pointer_position,
    ) {
        Some(MillerColumnAction::DragStarted(index))
    } else if response.double_clicked() {
        Some(MillerColumnAction::DoubleClicked(index))
    } else if response.secondary_clicked() {
        let pos = response
            .interact_pointer_pos()
            .or_else(|| ui.input(|i| i.pointer.interact_pos()))
            .unwrap_or(egui::Pos2::ZERO);
        Some(MillerColumnAction::SecondaryClicked(index, pos))
    } else if response.clicked() {
        Some(MillerColumnAction::Clicked(index))
    } else {
        None
    }
}

fn background_secondary_action(response: &egui::Response, ui: &Ui) -> Option<MillerColumnAction> {
    if !response.secondary_clicked() {
        return None;
    }

    let position = response
        .interact_pointer_pos()
        .or_else(|| ui.input(|input| input.pointer.interact_pos()))
        .unwrap_or(egui::Pos2::ZERO);
    Some(MillerColumnAction::EmptySecondaryClicked(position))
}
