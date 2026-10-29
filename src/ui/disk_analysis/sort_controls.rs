//! Visible, synchronized grouping and ordering controls for Largest items.

use crate::app::disk_analysis_sorting::{LargestColumn, LargestGrouping};
use crate::app::disk_analysis_state::DiskAnalysisState;
use eframe::egui;
use rust_i18n::t;

fn grouping_label(grouping: LargestGrouping) -> String {
    match grouping {
        LargestGrouping::Mixed => t!("disk_analysis.sort_mixed").to_string(),
        LargestGrouping::FoldersFirst => t!("disk_analysis.sort_folders_first").to_string(),
        LargestGrouping::FilesFirst => t!("disk_analysis.sort_files_first").to_string(),
    }
}

fn column_label(column: LargestColumn) -> String {
    match column {
        LargestColumn::Name => t!("disk_analysis.col_name"),
        LargestColumn::Path => t!("disk_analysis.col_path"),
        LargestColumn::Type => t!("disk_analysis.col_type"),
        LargestColumn::Logical => t!("disk_analysis.logical_size_short"),
        LargestColumn::Allocated => t!("disk_analysis.allocated_size"),
        LargestColumn::Difference => t!("disk_analysis.col_difference"),
        LargestColumn::Files => t!("disk_analysis.sort_file_count"),
    }
    .to_string()
}

const COMBO_WIDTH: f32 = 170.0;

/// Reserve the whole label/control pair before wrapping, not just the space
/// remaining on the current row (which can let a nested horizontal overflow).
fn control_group(ui: &mut egui::Ui, label: String, contents: impl FnOnce(&mut egui::Ui)) {
    let label_width = ui
        .painter()
        .layout_no_wrap(
            label.clone(),
            egui::TextStyle::Body.resolve(ui.style()),
            ui.visuals().text_color(),
        )
        .size()
        .x;
    let width = label_width + ui.spacing().item_spacing.x + COMBO_WIDTH;
    ui.allocate_ui_with_layout(
        egui::vec2(width, ui.spacing().interact_size.y),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.label(label);
            contents(ui);
        },
    );
}

pub(super) fn render(state: &mut DiskAnalysisState, ui: &mut egui::Ui) {
    let before = state.largest_sort;
    ui.horizontal_wrapped(|ui| {
        control_group(ui, t!("disk_analysis.sort_show").to_string(), |ui| {
            egui::ComboBox::from_id_salt("largest_grouping")
                .width(COMBO_WIDTH)
                .selected_text(grouping_label(state.largest_sort.grouping))
                .show_ui(ui, |ui| {
                    for grouping in [
                        LargestGrouping::Mixed,
                        LargestGrouping::FoldersFirst,
                        LargestGrouping::FilesFirst,
                    ] {
                        ui.selectable_value(
                            &mut state.largest_sort.grouping,
                            grouping,
                            grouping_label(grouping),
                        );
                    }
                });
        });
        control_group(ui, t!("disk_analysis.sort_by").to_string(), |ui| {
            egui::ComboBox::from_id_salt("largest_sort_column")
                .width(COMBO_WIDTH)
                .selected_text(column_label(state.largest_sort.column))
                .show_ui(ui, |ui| {
                    for column in LargestColumn::SORTABLE {
                        if ui
                            .selectable_value(
                                &mut state.largest_sort.column,
                                column,
                                column_label(column),
                            )
                            .changed()
                        {
                            state.largest_sort.ascending = column.default_ascending();
                        }
                    }
                });
        });
        control_group(ui, t!("disk_analysis.sort_order").to_string(), |ui| {
            let alphabetical = matches!(
                state.largest_sort.column,
                LargestColumn::Name | LargestColumn::Path
            );
            let ascending = if alphabetical {
                t!("disk_analysis.sort_a_z")
            } else {
                t!("disk_analysis.sort_smallest_first")
            };
            let descending = if alphabetical {
                t!("disk_analysis.sort_z_a")
            } else {
                t!("disk_analysis.sort_largest_first")
            };
            egui::ComboBox::from_id_salt("largest_sort_direction")
                .width(COMBO_WIDTH)
                .selected_text(if state.largest_sort.ascending {
                    ascending.to_string()
                } else {
                    descending.to_string()
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut state.largest_sort.ascending,
                        true,
                        ascending.to_string(),
                    );
                    ui.selectable_value(
                        &mut state.largest_sort.ascending,
                        false,
                        descending.to_string(),
                    );
                });
        });
    });
    if state.largest_sort != before {
        state.sync_query_jobs();
    }
    ui.add_space(4.0);
}

pub(super) fn header_suffix(state: &DiskAnalysisState, column: LargestColumn) -> &'static str {
    if column == LargestColumn::Type {
        match state.largest_sort.grouping {
            LargestGrouping::Mixed => "",
            LargestGrouping::FoldersFirst => " ↑",
            LargestGrouping::FilesFirst => " ↓",
        }
    } else if state.largest_sort.column == column {
        if state.largest_sort.ascending {
            " ↑"
        } else {
            " ↓"
        }
    } else {
        ""
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_indicators_follow_the_shared_sort_preferences() {
        let mut state = DiskAnalysisState::new();
        assert_eq!(header_suffix(&state, LargestColumn::Type), "");
        assert_eq!(header_suffix(&state, LargestColumn::Allocated), " ↓");
        state.update_largest_sort(LargestColumn::Type);
        state.update_largest_sort(LargestColumn::Logical);
        assert_eq!(header_suffix(&state, LargestColumn::Type), " ↑");
        assert_eq!(header_suffix(&state, LargestColumn::Logical), " ↓");
        assert_eq!(header_suffix(&state, LargestColumn::Allocated), "");
        state.update_largest_sort(LargestColumn::Type);
        state.update_largest_sort(LargestColumn::Logical);
        assert_eq!(header_suffix(&state, LargestColumn::Type), " ↓");
        assert_eq!(header_suffix(&state, LargestColumn::Logical), " ↑");
    }

    #[test]
    fn controls_fit_one_row_in_a_wide_panel_and_wrap_in_a_narrow_panel() {
        let height = |width| {
            let ctx = egui::Context::default();
            let mut state = DiskAnalysisState::new();
            let mut height = 0.0;
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.set_max_width(width);
                render(&mut state, ui);
                assert!(
                    ui.min_rect().width() <= width + 1.0,
                    "Controls exceed {width}: {:?}",
                    ui.min_rect()
                );
                height = ui.min_rect().height();
            });
            height
        };
        let wide = height(1100.0);
        assert!(wide < 40.0, "Wide controls should occupy one row: {wide}");
        assert!(height(360.0) > wide);
    }
}
