//! Bottom results panel of the disk analyzer: resizable, collapsible and
//! tabbed (Largest / Efficiency / Duplicates — plan section 8).
//!
//! Tables are projections over `Vec<u32>` node indices; names/paths are
//! materialized only for rows actually painted this frame.

use super::{duplicates, efficiency, largest_items};
use crate::app::disk_analysis_state::{DiskAnalysisState, ResultsTab};
use eframe::egui;
use rust_i18n::t;

const MIN_PANEL_H: f32 = 96.0;
const MAX_PANEL_H: f32 = 800.0;
const RESIZE_HANDLE_H: f32 = 7.0;
/// Tab strip + collapse chrome height while collapsed.
pub const PANEL_CHROME_H: f32 = 40.0;
/// Default row height of every virtualized table here.
pub const ROW_HEIGHT: f32 = 22.0;
/// Header row height of sortable tables.
pub const HEADER_ROW_H: f32 = 24.0;

fn panel_id(collapsed: bool) -> egui::Id {
    egui::Id::new(if collapsed {
        "disk_analysis_results_collapsed"
    } else {
        "disk_analysis_results_expanded"
    })
}

pub fn render_results_panel(state: &mut DiskAnalysisState, ui: &mut egui::Ui) {
    let collapsed = state.results_collapsed;
    // Keep collapsed and expanded geometry in separate egui states. Reusing
    // one ID makes the collapsed chrome size replace the user's resized height.
    let panel_height = if collapsed {
        PANEL_CHROME_H
    } else {
        state.results_height.clamp(MIN_PANEL_H, MAX_PANEL_H)
    };
    let panel = egui::Panel::bottom(panel_id(collapsed))
        .show_separator_line(false)
        .resizable(false)
        .exact_size(panel_height)
        .frame(
            egui::Frame::NONE
                .fill(ui.visuals().panel_fill)
                .inner_margin(egui::Margin::ZERO),
        );

    panel.show(ui, |ui| {
        let item_spacing_y = ui.spacing().item_spacing.y;
        ui.spacing_mut().item_spacing.y = 0.0;
        if !collapsed {
            render_resize_handle(state, ui);
        }
        render_panel_chrome(state, ui);
        if !collapsed {
            egui::Frame::NONE
                .inner_margin(egui::Margin::symmetric(8, 4))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = item_spacing_y;
                    match state.results_tab {
                        ResultsTab::Largest => largest_items::render(state, ui),
                        ResultsTab::Efficiency => efficiency::render(state, ui),
                        ResultsTab::Duplicates => duplicates::render(state, ui),
                    }
                });
        }
    });
}

fn render_resize_handle(state: &mut DiskAnalysisState, ui: &mut egui::Ui) {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), RESIZE_HANDLE_H),
        egui::Sense::drag(),
    );
    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }
    if response.dragged() {
        let delta = ui.input(|input| input.pointer.delta().y);
        state.results_height = (state.results_height - delta).clamp(MIN_PANEL_H, MAX_PANEL_H);
        ui.ctx().request_repaint();
    }
    ui.painter().hline(
        rect.x_range(),
        rect.top() + 1.0,
        egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
    );
}

fn render_panel_chrome(state: &mut DiskAnalysisState, ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), PANEL_CHROME_H),
        egui::Sense::hover(),
    );
    let (fill, separator) = crate::ui::theme::viewer_bar_colors(ui.visuals().dark_mode);
    ui.painter().rect_filled(rect, 0.0, fill);
    ui.painter().hline(
        rect.x_range(),
        rect.bottom(),
        egui::Stroke::new(1.0, separator),
    );

    let content_rect = rect.shrink2(egui::vec2(10.0, 6.0));
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(content_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for (tab, label) in [
                (ResultsTab::Largest, t!("disk_analysis.tab_largest")),
                (ResultsTab::Efficiency, t!("disk_analysis.tab_efficiency")),
                (ResultsTab::Duplicates, t!("disk_analysis.tab_duplicates")),
            ] {
                if ui
                    .selectable_label(state.results_tab == tab, label.to_string())
                    .clicked()
                {
                    state.results_tab = tab;
                }
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let glyph = if state.results_collapsed { "^" } else { "v" };
                let hint = if state.results_collapsed {
                    t!("disk_analysis.panel_expand")
                } else {
                    t!("disk_analysis.panel_collapse")
                };
                if ui.button(glyph).on_hover_text(hint.to_string()).clicked() {
                    state.results_collapsed = !state.results_collapsed;
                }
            });
        },
    );
}

// ---------------------------------------------------------------------------
// Shared cell painting used by the Largest and Efficiency tables.
// ---------------------------------------------------------------------------

/// Paints one truncated table cell starting at `*x` with the given slot
/// width; advances `*x`. Right-aligned cells truncate from the left so
/// numeric tails stay visible.
pub(crate) fn paint_cell(
    painter: &egui::Painter,
    row_rect: egui::Rect,
    x: &mut f32,
    width: f32,
    text: &str,
    align_right: bool,
    color: egui::Color32,
) {
    const PAD: f32 = 6.0;
    let max_w = width - PAD * 2.0;
    if max_w <= 4.0 || text.is_empty() {
        *x += width;
        return;
    }
    // Character-count approximation keeps per-cell cost trivial; exact
    // measurement would allocate a galley per candidate width.
    const CHAR_W: f32 = 7.0;
    let max_chars = ((max_w / CHAR_W).floor() as usize).max(2);
    let count = text.chars().count();
    let display: String = if count <= max_chars {
        text.to_string()
    } else if align_right {
        let mut s: String = text.chars().skip(count + 1 - max_chars).collect();
        s.insert(0, '…');
        s
    } else {
        let mut s: String = text.chars().take(max_chars.saturating_sub(1)).collect();
        s.push('…');
        s
    };

    let font_id = egui::FontId::proportional(12.5);
    let galley = painter.layout_no_wrap(display, font_id, color);
    let pos = if align_right {
        egui::pos2(
            *x + width - PAD - galley.size().x,
            row_rect.center().y - galley.size().y / 2.0,
        )
    } else {
        egui::pos2(*x + PAD, row_rect.center().y - galley.size().y / 2.0)
    };
    painter.galley(pos, galley, color);
    *x += width;
}

/// Context menu shared by result rows: **Open in main app** everywhere, plus
/// **Copy path** on the Duplicates report (plan sections 4 and 7).
pub(crate) fn attach_row_menu(
    state: &mut DiskAnalysisState,
    ui: &mut egui::Ui,
    resp: &egui::Response,
    idx: u32,
    include_copy_path: bool,
) -> bool {
    use egui::SetOpenCommand;
    let menu_id = egui::Id::new("disk_analysis_row_menu").with(idx);
    let open_cmd = if resp.secondary_clicked() {
        Some(SetOpenCommand::Bool(true))
    } else if resp.clicked() {
        Some(SetOpenCommand::Bool(false))
    } else {
        None
    };
    let mut activated = false;
    let menu = egui::Popup::context_menu(resp)
        .id(menu_id)
        .open_memory(open_cmd);
    if let Some(menu_resp) = menu.show(|ui| {
        let open = ui
            .button(t!("disk_analysis.open_in_main_app").to_string())
            .clicked();
        let copy = if include_copy_path {
            ui.button(t!("disk_analysis.copy_path").to_string())
                .clicked()
        } else {
            false
        };
        (open, copy)
    }) {
        let (open_clicked, copy_clicked) = menu_resp.inner;
        if open_clicked || copy_clicked {
            if let Some(model) = state.model.clone() {
                let target = model.path_of(idx);
                let is_dir = model.nodes[idx as usize].is_dir;
                if open_clicked {
                    crate::disk_analyzer::open_in_main::open_path_in_main_app(&target, is_dir);
                } else {
                    let _ = crate::application::file_operations::copy_path_to_clipboard(
                        std::path::Path::new(&target),
                    );
                }
            }
            egui::Popup::close_id(ui.ctx(), menu_id);
            activated = true;
        }
    }
    activated
}

#[cfg(test)]
mod tests {
    use super::panel_id;

    #[test]
    fn collapsed_panel_does_not_overwrite_expanded_geometry() {
        assert_ne!(panel_id(false), panel_id(true));
        assert_eq!(panel_id(false), panel_id(false));
    }
}
