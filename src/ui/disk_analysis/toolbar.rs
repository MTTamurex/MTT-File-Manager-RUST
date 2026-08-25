//! Analyzer toolbar: metric selector, subtree search box and the combined
//! filter controls (plan sections 2, 3 and 5).
//!
//! Everything here only mutates draft state; actual recomputation happens in
//! the query worker after the debounce handled by
//! [`DiskAnalysisState::sync_query_jobs`].

use crate::app::disk_analysis_model::FileCategory;
use crate::app::disk_analysis_query::{DiskAnalysisFilter, SizeMetric};
use crate::app::disk_analysis_state::DiskAnalysisState;
use crate::ui::disk_analysis::category_label;
use eframe::egui;
use rust_i18n::t;

pub const SEARCH_TEXT_EDIT_ID: &str = "disk_analysis_search_edit";

/// Height of the toolbar row so callers can lay out around it.
pub fn render_toolbar(state: &mut DiskAnalysisState, ui: &mut egui::Ui) {
    handle_search_shortcuts(state, ui);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;

        // ---- Metric segmented control ----------------------------------
        let metric_before = state.metric;
        let metric_after = render_metric_selector(state, ui);
        if metric_after != metric_before {
            // Largest rows depend on the weight basis; reschedule them.
            state.sync_query_jobs();
        }

        ui.separator();

        // ---- Search field ----------------------------------------------
        render_search_field(state, ui);

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let toggle_label = if state.filters_visible {
                t!("disk_analysis.filters_hide").to_string()
            } else {
                t!("disk_analysis.filters_show").to_string()
            };
            if ui
                .selectable_label(state.filters_visible, toggle_label)
                .clicked()
            {
                state.filters_visible = !state.filters_visible;
            }
        });
    });

    if state.filters_visible {
        ui.add_space(4.0);
        render_filter_row(state, ui);
    }

    ui.add_space(4.0);
    render_search_results_popup(state, ui);
}

fn render_metric_selector(state: &mut DiskAnalysisState, ui: &mut egui::Ui) -> SizeMetric {
    let labels = [
        (SizeMetric::Allocated, t!("disk_analysis.metric_allocated")),
        (SizeMetric::Logical, t!("disk_analysis.metric_logical")),
        (SizeMetric::FileCount, t!("disk_analysis.metric_files")),
    ];
    let mut selected = state.metric;
    ui.horizontal(|ui| {
        for (metric, label) in labels {
            if ui
                .selectable_label(state.metric == metric, label.to_string())
                .clicked()
            {
                selected = metric;
            }
        }
    });
    if state.metric != selected {
        state.metric = selected;
        state.largest_sort_column = match selected {
            SizeMetric::Allocated => crate::app::disk_analysis_state::LargestColumn::Allocated,
            SizeMetric::Logical => crate::app::disk_analysis_state::LargestColumn::Logical,
            SizeMetric::FileCount => crate::app::disk_analysis_state::LargestColumn::Files,
        };
        state.largest_sort_asc = false;
    }
    selected
}

fn search_edit_id() -> egui::Id {
    egui::Id::new(SEARCH_TEXT_EDIT_ID)
}

fn handle_search_shortcuts(state: &mut DiskAnalysisState, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let ctrl_f = ctx.input(|i| {
        i.key_pressed(egui::Key::F) && i.modifiers.ctrl && !i.modifiers.shift && !i.modifiers.alt
    });
    if ctrl_f {
        ctx.memory_mut(|mem| mem.request_focus(search_edit_id()));
        state.search_open = !state.search_text.trim().is_empty();
    }
}

fn render_search_field(state: &mut DiskAnalysisState, ui: &mut egui::Ui) {
    let id = search_edit_id();
    let hint = t!("disk_analysis.search_placeholder").to_string();
    let edit = egui::TextEdit::singleline(&mut state.search_text)
        .id(id)
        .desired_width(240.0)
        .hint_text(hint);
    let response = ui.add(edit);
    if response.changed() {
        state.mark_search_changed();
    }
    if state.search_text.trim().is_empty() {
        state.search_open = false;
    }

    // Enter activates the highlighted result; arrows navigate.
    let focused = ui.ctx().memory(|mem| mem.has_focus(id));
    if focused && !state.search_results.is_empty() {
        let (up, down, enter) = ui.ctx().input(|i| {
            (
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::Enter),
            )
        });
        if up {
            state.search_selected = state.search_selected.saturating_sub(1);
        }
        if down && state.search_selected + 1 < state.search_results.len() {
            state.search_selected += 1;
        }
        if up || down {
            ui.ctx().request_repaint();
        }
        if enter {
            let idx =
                state.search_results[state.search_selected.min(state.search_results.len() - 1)];
            activate_result(state, idx);
            ui.ctx().memory_mut(|mem| mem.surrender_focus(id));
        }
    }
}

/// Activate one search hit per plan section 5: directories drill in, files
/// open their parent folder and get a persistent highlight; the bottom panel
/// scrolls to the matching row when present.
pub fn activate_result(state: &mut DiskAnalysisState, idx: u32) {
    let is_dir = state
        .model
        .as_ref()
        .map(|m| m.nodes[idx as usize].is_dir)
        .unwrap_or(false);
    if is_dir {
        state.navigate_to(idx);
        state.selected = None;
    } else {
        state.reveal_file(idx);
        // Ask the Largest tab to scroll this row into view next frame.
        if let Some(pos) = state.largest_rows.iter().position(|&r| r == idx) {
            state.largest_scroll_to_row = Some(pos);
        }
    }
    state.search_text.clear();
    state.mark_search_changed();
    state.search_open = false;
}

fn render_filter_row(state: &mut DiskAnalysisState, ui: &mut egui::Ui) {
    ui.horizontal_wrapped(|ui| {
        ui.label(t!("disk_analysis.filter_category").to_string());
        let all_mask = DiskAnalysisFilter::all_categories_mask();
        if ui
            .selectable_label(
                state.filter_categories_mask == all_mask,
                t!("disk_analysis.category_all").to_string(),
            )
            .clicked()
        {
            state.filter_categories_mask = all_mask;
            state.mark_filter_changed();
        }
        for category in FileCategory::ALL {
            let bit = 1u8 << category.index();
            let selected = state.filter_categories_mask & bit != 0;
            if ui
                .selectable_label(selected, category_label(category))
                .clicked()
            {
                state.filter_categories_mask ^= bit;
                state.mark_filter_changed();
            }
        }
    });
    ui.add_space(4.0);

    ui.horizontal_wrapped(|ui| {
        ui.label(t!("disk_analysis.filter_extension").to_string());
        let ext_response = ui.add(
            egui::TextEdit::singleline(&mut state.filter_extensions_text)
                .desired_width(120.0)
                .hint_text("pdf, mp4"),
        );
        if ext_response.changed() {
            state.mark_filter_changed();
            state.sync_query_jobs();
        }

        ui.separator();
        ui.label(t!("disk_analysis.filter_min_size").to_string());
        if size_field(ui, &mut state.filter_min_size_text).changed() {
            state.mark_filter_changed();
            state.sync_query_jobs();
        }
        ui.label(t!("disk_analysis.filter_max_size").to_string());
        if size_field(ui, &mut state.filter_max_size_text).changed() {
            state.mark_filter_changed();
            state.sync_query_jobs();
        }

        ui.separator();
        let base_logical = ui.selectable_label(
            state.filter_size_base == crate::app::disk_analysis_query::FilterSizeBase::Logical,
            t!("disk_analysis.logical_size").to_string(),
        );
        let base_allocated = ui.selectable_label(
            state.filter_size_base == crate::app::disk_analysis_query::FilterSizeBase::Allocated,
            t!("disk_analysis.allocated_size").to_string(),
        );
        if base_logical.clicked()
            && state.filter_size_base != crate::app::disk_analysis_query::FilterSizeBase::Logical
        {
            state.filter_size_base = crate::app::disk_analysis_query::FilterSizeBase::Logical;
            state.mark_filter_changed();
            state.sync_query_jobs();
        }
        if base_allocated.clicked()
            && state.filter_size_base != crate::app::disk_analysis_query::FilterSizeBase::Allocated
        {
            state.filter_size_base = crate::app::disk_analysis_query::FilterSizeBase::Allocated;
            state.mark_filter_changed();
            state.sync_query_jobs();
        }

        ui.separator();
        if ui
            .button(t!("disk_analysis.filter_clear").to_string())
            .clicked()
        {
            state.filter_extensions_text.clear();
            state.filter_min_size_text.clear();
            state.filter_max_size_text.clear();
            state.filter_categories_mask = DiskAnalysisFilter::all_categories_mask();
            state.filter_size_base = Default::default();
            state.mark_filter_changed();
            state.sync_query_jobs();
        }
    });

    // Human-readable status of the effective filter.
    if state.filter.is_active() {
        let mut parts: Vec<String> = Vec::new();
        if state.filter.categories_mask != DiskAnalysisFilter::all_categories_mask() {
            let categories = FileCategory::ALL
                .into_iter()
                .filter(|category| state.filter.categories_mask & (1 << category.index()) != 0)
                .map(category_label)
                .collect::<Vec<_>>()
                .join(", ");
            parts.push(format!(
                "{}: {}",
                t!("disk_analysis.filter_category"),
                categories
            ));
        }
        if !state.filter.extensions.is_empty() {
            parts.push(format!(
                "{}: {}",
                t!("disk_analysis.filter_extension"),
                state.filter.extensions.join(", ")
            ));
        }
        if let Some(min) = state.filter.min_size {
            parts.push(format!(
                "≥ {}",
                crate::infrastructure::windows::formatting::format_size(min)
            ));
        }
        if let Some(max) = state.filter.max_size {
            parts.push(format!(
                "≤ {}",
                crate::infrastructure::windows::formatting::format_size(max)
            ));
        }
        ui.label(
            egui::RichText::new(parts.join(" · "))
                .small()
                .color(super::panel_secondary_text_color(ui)),
        );
    }
}

fn size_field(ui: &mut egui::Ui, text: &mut String) -> egui::Response {
    ui.add(
        egui::TextEdit::singleline(text)
            .desired_width(90.0)
            .hint_text("bytes"),
    )
}

/// Dropdown list of search results anchored under the toolbar area.
fn render_search_results_popup(state: &mut DiskAnalysisState, ui: &mut egui::Ui) {
    if !state.search_open || state.search_results.is_empty() {
        return;
    }
    let model = match state.model.clone() {
        Some(m) => m,
        None => return,
    };
    let anchor = ui.max_rect().left_top();
    let row_height = 22.0;
    let visible_rows = state.search_results.len().min(10) as f32;
    let popup_height = visible_rows * row_height + 12.0;
    let popup_width = ui.available_width().min(560.0);

    egui::Area::new(egui::Id::new("disk_analysis_search_results"))
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::LEFT_TOP, [anchor.x, anchor.y + 4.0])
        .show(ui.ctx(), |ui: &mut egui::Ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_size(egui::vec2(popup_width, popup_height));
                egui::ScrollArea::vertical()
                    .id_salt("disk_analysis_search_results_scroll")
                    .show_rows(ui, row_height, state.search_results.len(), |ui, range| {
                        for row in range {
                            let idx = state.search_results[row];
                            let node = &model.nodes[idx as usize];
                            let selected_row = row
                                == state
                                    .search_selected
                                    .min(state.search_results.len().saturating_sub(1));
                            let label = format!(
                                "{}  {}",
                                node.name,
                                if node.is_dir {
                                    String::new()
                                } else {
                                    format!(
                                        "({})",
                                        crate::infrastructure::windows::formatting::format_size(
                                            node.size
                                        )
                                    )
                                }
                            );
                            let resp = ui.allocate_response(
                                egui::vec2(ui.available_width(), row_height),
                                egui::Sense::click(),
                            );
                            ui.painter().rect_filled(
                                resp.rect,
                                3.0,
                                if selected_row {
                                    ui.visuals().selection.bg_fill
                                } else if resp.hovered() {
                                    ui.visuals().widgets.hovered.bg_fill
                                } else {
                                    egui::Color32::TRANSPARENT
                                },
                            );
                            ui.painter().text(
                                resp.rect.left_center() + egui::vec2(6.0, 0.0),
                                egui::Align2::LEFT_CENTER,
                                label,
                                egui::FontId::proportional(13.0),
                                super::analyzer_text_color(ui),
                            );
                            if resp.clicked() {
                                activate_result(state, idx);
                                ui.ctx()
                                    .memory_mut(|mem| mem.surrender_focus(search_edit_id()));
                            } else if resp.hovered() {
                                state.search_selected = row;
                            }
                        }
                    });
            });
        });
}
