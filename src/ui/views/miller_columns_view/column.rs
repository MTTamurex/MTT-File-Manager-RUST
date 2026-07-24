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
use crate::ui::views::rectangle_selection::{
    paint_overlay, ListRectangleMetrics, RectangleSelectionFrame, RectangleSelectionMetrics,
    RectangleSelectionSource, RectangleSelectionState,
};

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

pub struct MillerColumnOutput {
    pub action: Option<MillerColumnAction>,
    pub rectangle_selection_frame: RectangleSelectionFrame,
}

/// Borrowed context needed to render one ancestor column.
pub struct MillerColumnContext<'a> {
    pub items: &'a [FileEntry],
    pub directory: &'a Path,
    /// The child directory (from this column) that leads to the next column.
    pub selected_child: Option<&'a Path>,
    /// The currently previewed file/selection path (for highlight).
    pub selected_file: Option<&'a Path>,
    pub multi_selection: &'a crate::ui::cache::FxHashSet<PathBuf>,
    pub rectangle_selection_state: Option<&'a RectangleSelectionState>,
    pub listing_id: usize,
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
) -> MillerColumnOutput {
    let dark = ui.visuals().dark_mode;
    let rect = ui.max_rect();
    let column_id = ui.make_persistent_id(id_salt);
    let background_response =
        ui.interact(rect, column_id.with("background"), Sense::click_and_drag());

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
        return MillerColumnOutput {
            action: background_secondary_action(&background_response, ui),
            rectangle_selection_frame: RectangleSelectionFrame::default(),
        };
    }

    let mut action = None;
    let mut rectangle_start = None;
    ui.spacing_mut().item_spacing.y = 0.0;
    let scroll_output = egui::ScrollArea::vertical()
        .id_salt(column_id)
        .auto_shrink([false, false])
        .drag_to_scroll(false)
        .show_rows(ui, COL_ROW_HEIGHT, ctx.items.len(), |ui, row_range| {
            for index in row_range {
                let Some(item) = ctx.items.get(index) else {
                    continue;
                };
                if let Some(a) = render_row(ui, index, item, ctx, dark, &mut rectangle_start) {
                    action = Some(a);
                }
            }
        });

    if action.is_none() && background_response.drag_started() {
        rectangle_start = ui.input(|input| input.pointer.press_origin());
    }

    let source = RectangleSelectionSource::MillerAncestor {
        directory: ctx.directory.to_path_buf(),
        listing_id: ctx.listing_id,
    };
    let current_scroll_y = scroll_output.state.offset.y;
    let max_scroll_y = (scroll_output.content_size.y - scroll_output.inner_rect.height()).max(0.0);
    let mut rectangle_selection_frame = RectangleSelectionFrame::default();
    rectangle_selection_frame.begin(
        scroll_output.inner_rect,
        0.0,
        current_scroll_y,
        0.0,
        max_scroll_y,
        Some(RectangleSelectionMetrics::List(ListRectangleMetrics {
            count: ctx.items.len(),
            row_height: COL_ROW_HEIGHT,
            content_width: scroll_output.inner_rect.width(),
            content_height: ctx.items.len() as f32 * COL_ROW_HEIGHT,
        })),
    );
    rectangle_selection_frame.source = source;
    if let Some(start) = rectangle_start {
        rectangle_selection_frame.request_start(start);
    }

    if let Some(state) = ctx.rectangle_selection_state {
        paint_overlay(ui, state, scroll_output.inner_rect, 0.0, current_scroll_y);
    }

    MillerColumnOutput {
        action: action.or_else(|| background_secondary_action(&background_response, ui)),
        rectangle_selection_frame,
    }
}

fn render_row(
    ui: &mut Ui,
    index: usize,
    item: &FileEntry,
    ctx: &mut MillerColumnContext,
    dark: bool,
    rectangle_start: &mut Option<egui::Pos2>,
) -> Option<MillerColumnAction> {
    let width = ui.available_width();
    let (row_rect, response) =
        ui.allocate_exact_size(egui::vec2(width, COL_ROW_HEIGHT), Sense::click_and_drag());

    let is_drop_candidate = ctx.is_item_dragging && response.contains_pointer() && item.is_dir;
    if is_drop_candidate {
        *ctx.drop_target = Some(item.path.clone());
    }

    if !ui.is_rect_visible(row_rect) {
        return interaction(index, item, row_rect, &response, ui, rectangle_start);
    }

    let is_selected = ctx
        .rectangle_selection_state
        .is_some_and(|state| state.preview_contains(index))
        || ctx.multi_selection.contains(&item.path)
        || ctx.selected_child.is_some_and(|p| p == item.path.as_path())
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

    interaction(index, item, row_rect, &response, ui, rectangle_start)
}

fn interaction(
    index: usize,
    item: &FileEntry,
    row_rect: Rect,
    response: &egui::Response,
    ui: &Ui,
    rectangle_start: &mut Option<egui::Pos2>,
) -> Option<MillerColumnAction> {
    let (press_origin, pointer_position) =
        ui.input(|input| (input.pointer.press_origin(), input.pointer.hover_pos()));
    if crate::ui::views::common::should_start_item_drag(
        response.drag_started(),
        response.dragged(),
        response.is_pointer_button_down_on(),
        press_origin,
        pointer_position,
    ) {
        if press_origin
            .is_some_and(|origin| row_content_contains_pointer(ui, item, row_rect, origin))
        {
            Some(MillerColumnAction::DragStarted(index))
        } else {
            *rectangle_start = press_origin;
            None
        }
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

fn row_content_contains_pointer(
    ui: &Ui,
    item: &FileEntry,
    row_rect: Rect,
    point: egui::Pos2,
) -> bool {
    let icon_x = row_rect.left() + LEFT_PAD;
    let icon_rect = Rect::from_center_size(
        egui::pos2(icon_x + ICON_SIZE / 2.0, row_rect.center().y),
        egui::vec2(ICON_SIZE, ICON_SIZE),
    );
    if icon_rect.expand(2.0).contains(point) {
        return true;
    }

    let text_x = icon_x + ICON_SIZE + 6.0;
    let font = FontId::proportional(13.0);
    let max_text_w = (row_rect.right() - text_x - CHEVRON_W - 4.0).max(0.0);
    let display = truncate_text_for_column(&item.name, max_text_w, &font, ui);
    let text_width = ui.fonts(|fonts| {
        fonts
            .layout_no_wrap(display, font, Color32::WHITE)
            .rect
            .width()
    });
    let text_rect = Rect::from_min_size(
        egui::pos2(text_x, row_rect.top() + 3.0),
        egui::vec2(text_width.min(max_text_w), COL_ROW_HEIGHT - 6.0),
    );
    if text_rect.expand(2.0).contains(point) {
        return true;
    }

    item.is_dir
        && Rect::from_min_size(
            egui::pos2(row_rect.right() - CHEVRON_W, row_rect.top()),
            egui::vec2(CHEVRON_W, COL_ROW_HEIGHT),
        )
        .contains(point)
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
