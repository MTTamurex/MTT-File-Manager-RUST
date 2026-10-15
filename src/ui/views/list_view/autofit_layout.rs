//! Track the geometry used by the last successful fit of each physical pane.

use eframe::egui::{Id, Ui};

fn fitted_width_id(ui: &Ui) -> Id {
    ui.id().with("list_column_fitted_width")
}

pub(crate) fn needs_refit(ui: &Ui, pending: bool) -> bool {
    let previous_width = ui.data_mut(|data| data.get_temp::<f32>(fitted_width_id(ui)));
    pending || previous_width.is_none_or(|width| (width - ui.available_width()).abs() > 1.0)
}

pub(crate) fn record_fitted_width(ui: &Ui) {
    ui.data_mut(|data| data.insert_temp(fitted_width_id(ui), ui.available_width()));
}

#[cfg(test)]
#[path = "autofit_layout_tests.rs"]
mod tests;
