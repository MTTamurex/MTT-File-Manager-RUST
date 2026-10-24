//! Compact icon + name renderer for a single Miller's Columns ancestor column.
//!
//! The focused (rightmost) column is rendered by the normal details list view;
//! this renderer is used only for the ancestor columns to its left. It supports
//! selection highlight, inline rename, hover, and click / double-click /
//! right-click, all reported back to the bridge which applies path-based actions.

use eframe::egui::{self, Color32, FontId, Rect, Sense, Ui};
use std::path::{Path, PathBuf};

use crate::domain::file_entry::FileEntry;
use crate::ui::icon_loader::IconLoader;
use crate::ui::theme;
use crate::ui::views::list_view::truncate_text_for_column;
use crate::ui::views::rectangle_selection::{RectangleSelectionFrame, RectangleSelectionState};

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
    EmptyClicked,
    EmptySecondaryClicked(egui::Pos2),
}
pub struct MillerColumnOutput {
    pub action: Option<MillerColumnAction>,
    pub toggled_group: Option<crate::application::grouping::GroupKey>,
    pub rectangle_selection_frame: RectangleSelectionFrame,
    pub rename_update: Option<MillerRenameUpdate>,
}

pub struct MillerRenameUpdate {
    pub path: PathBuf,
    pub text: String,
    pub commit: bool,
}

/// Borrowed context needed to render one ancestor column.
pub struct MillerColumnContext<'a> {
    pub items: &'a [FileEntry],
    pub group_projection: &'a crate::application::grouping::GroupProjection,
    pub collapsed_groups: &'a crate::ui::cache::FxHashSet<crate::application::grouping::GroupKey>,
    pub directory: &'a Path,
    /// The child directory (from this column) that leads to the next column.
    pub selected_child: Option<&'a Path>,
    /// The currently previewed file/selection path (for highlight).
    pub selected_file: Option<&'a Path>,
    pub multi_selection: &'a crate::ui::cache::FxHashSet<PathBuf>,
    pub renaming_state: Option<&'a (PathBuf, String)>,
    pub focus_rename: bool,
    pub rectangle_selection_state: Option<&'a RectangleSelectionState>,
    pub icon_loader: &'a mut IconLoader,
    pub folder_icon: Option<&'a egui::TextureHandle>,
    pub loading_icons: &'a crate::ui::icon_loader::IconLoadTracker,
    pub failed_icons: &'a crate::ui::icon_loader::IconFailureTracker,
    pub icon_requests: &'a mut Vec<PathBuf>,
    pub is_item_dragging: bool,
    pub drop_target: &'a mut Option<PathBuf>,
    pub external_drop_pointer_pos: Option<egui::Pos2>,
    pub is_loading: bool,
    pub scroll_interpolation_enabled: bool,
}

pub(super) fn render_row(
    ui: &mut Ui,
    index: usize,
    item: &FileEntry,
    ctx: &mut MillerColumnContext,
    dark: bool,
    rectangle_start: &mut Option<egui::Pos2>,
    rename_update: &mut Option<MillerRenameUpdate>,
) -> Option<MillerColumnAction> {
    let width = ui.available_width();
    let row_height = crate::ui::fonts::layout::row_height(ui, COL_ROW_HEIGHT, 13.0, 6.0);
    let (row_rect, response) =
        ui.allocate_exact_size(egui::vec2(width, row_height), Sense::click_and_drag());
    let is_selected = crate::ui::views::common::effective_item_selection(
        ctx.multi_selection.contains(&item.path),
        ctx.rectangle_selection_state
            .map(|state| state.preview_contains(index)),
    );

    let pointer_over_row = ctx
        .external_drop_pointer_pos
        .map(|position| row_rect.contains(position))
        .unwrap_or_else(|| response.contains_pointer());
    let is_drop_candidate = ctx.is_item_dragging && pointer_over_row && item.is_dir;
    if is_drop_candidate {
        *ctx.drop_target = Some(item.path.clone());
    }

    if !ui.is_rect_visible(row_rect) {
        return interaction(
            index,
            item,
            row_rect,
            &response,
            ui,
            rectangle_start,
            is_selected,
        );
    }

    let is_navigation_child = ctx.selected_child.is_some_and(|p| p == item.path.as_path());
    let is_focused = ctx.selected_file.is_some_and(|p| p == item.path.as_path());
    let dim = if item.is_hidden { 0.5 } else { 1.0 };

    crate::ui::list_stripes::paint(ui, row_rect, index);

    if is_navigation_child && !is_selected {
        ui.painter()
            .rect_filled(row_rect, 0.0, theme::selection_hover_color(ui.ctx()));
    }

    let mut visual_rect = row_rect;
    visual_rect.max.x -= 4.0;
    if is_selected {
        ui.painter().rect_stroke(
            visual_rect,
            4.0,
            egui::Stroke::new(2.0, theme::accent_color(ui.ctx())),
            egui::StrokeKind::Inside,
        );
    } else if response.hovered() || is_focused {
        ui.painter().rect_stroke(
            visual_rect,
            4.0,
            egui::Stroke::new(1.0, theme::accent_color(ui.ctx()).gamma_multiply(0.35)),
            egui::StrokeKind::Inside,
        );
    }

    crate::ui::clipboard_feedback::paint(ui, visual_rect, &item.path);

    if is_drop_candidate {
        ui.painter().rect_stroke(
            row_rect.shrink(1.0),
            4.0,
            egui::Stroke::new(2.0, theme::accent_color(ui.ctx())),
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
        && !ctx
            .loading_icons
            .contains(&item.path, crate::domain::file_entry::IconSize::Large)
        && !ctx
            .failed_icons
            .contains(&item.path, crate::domain::file_entry::IconSize::Large)
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
            Color32::WHITE.gamma_multiply(
                dim * crate::ui::clipboard_feedback::icon_opacity(ui.ctx(), &item.path),
            ),
        );
    }

    // Name text (truncated to fit).
    let text_x = icon_x + ICON_SIZE + 6.0;
    if let Some((_, rename_text)) = ctx.renaming_state.filter(|(path, _)| *path == item.path) {
        let mut text = rename_text.clone();
        let edit_rect = Rect::from_min_max(
            egui::pos2(text_x, row_rect.top() + 2.0),
            egui::pos2(row_rect.right() - 5.0, row_rect.bottom() - 2.0),
        );
        let mut commit = false;
        ui.scope_builder(egui::UiBuilder::new().max_rect(edit_rect), |ui| {
            let response = ui.add(
                egui::TextEdit::singleline(&mut text)
                    .id_source(("rename_input_miller", &item.path)),
            );
            let text_menu_active = crate::ui::components::inline_rename::add_text_context_menu(
                ui,
                &response,
                &mut text,
                ctx.focus_rename,
            );
            if ctx.focus_rename {
                response.request_focus();
                if let Some(mut state) =
                    egui::widgets::text_edit::TextEditState::load(ui.ctx(), response.id)
                {
                    let char_count = text.chars().count();
                    let select_end = if item.is_dir {
                        char_count
                    } else {
                        text.rfind('.')
                            .map(|byte_pos| text[..byte_pos].chars().count())
                            .filter(|&position| position > 0)
                            .unwrap_or(char_count)
                    };
                    state
                        .cursor
                        .set_char_range(Some(egui::text::CCursorRange::two(
                            egui::text::CCursor::new(0),
                            egui::text::CCursor::new(select_end),
                        )));
                    state.store(ui.ctx(), response.id);
                }
            }
            commit = (response.has_focus()
                && ui.input(|input| input.key_pressed(egui::Key::Enter)))
                || (response.lost_focus() && !text_menu_active.menu_active);
        });
        *rename_update = Some(MillerRenameUpdate {
            path: item.path.clone(),
            text,
            commit,
        });
        return None;
    }

    let text_color = if is_selected {
        theme::selection_text_color(ui.ctx())
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

    interaction(
        index,
        item,
        row_rect,
        &response,
        ui,
        rectangle_start,
        is_selected,
    )
}

fn interaction(
    index: usize,
    item: &FileEntry,
    row_rect: Rect,
    response: &egui::Response,
    ui: &Ui,
    rectangle_start: &mut Option<egui::Pos2>,
    is_selected: bool,
) -> Option<MillerColumnAction> {
    let (press_origin, pointer_position, primary_button_down) = ui.input(|input| {
        (
            input.pointer.press_origin(),
            input.pointer.hover_pos(),
            input.pointer.button_down(egui::PointerButton::Primary),
        )
    });
    crate::ui::new_tab_request::item_middle_click(response, item, false);
    if crate::ui::views::common::should_start_item_drag(
        response.drag_started_by(egui::PointerButton::Primary),
        response.dragged_by(egui::PointerButton::Primary),
        primary_button_down,
        response.is_pointer_button_down_on() && primary_button_down,
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
        if crate::ui::views::common::secondary_click_targets_item(
            is_selected,
            row_content_contains_pointer(ui, item, row_rect, pos),
        ) {
            Some(MillerColumnAction::SecondaryClicked(index, pos))
        } else {
            Some(MillerColumnAction::EmptySecondaryClicked(pos))
        }
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
    let text_width = ui
        .painter()
        .layout_no_wrap(display, font, Color32::WHITE)
        .rect
        .width();
    let text_rect = Rect::from_min_size(
        egui::pos2(text_x, row_rect.top() + 3.0),
        egui::vec2(text_width.min(max_text_w), row_rect.height() - 6.0),
    );
    if text_rect.expand(2.0).contains(point) {
        return true;
    }

    item.is_dir
        && Rect::from_min_size(
            egui::pos2(row_rect.right() - CHEVRON_W, row_rect.top()),
            egui::vec2(CHEVRON_W, row_rect.height()),
        )
        .contains(point)
}

pub(super) fn background_action(response: &egui::Response, ui: &Ui) -> Option<MillerColumnAction> {
    if response.secondary_clicked() {
        let position = response
            .interact_pointer_pos()
            .or_else(|| ui.input(|input| input.pointer.interact_pos()))
            .unwrap_or(egui::Pos2::ZERO);
        Some(MillerColumnAction::EmptySecondaryClicked(position))
    } else if response.clicked() {
        Some(MillerColumnAction::EmptyClicked)
    } else {
        None
    }
}
