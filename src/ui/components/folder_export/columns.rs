use super::*;
use crate::domain::list_columns::ColumnId;

pub(super) fn render(ui: &mut egui::Ui, state: &mut FolderExportState) {
    let mut movement = None;
    let count = state.options.columns.len();
    egui::Frame::group(ui.style())
        .inner_margin(6.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical()
                .id_salt("export_columns")
                .max_height(132.0)
                .show(ui, |ui| {
                    for (index, column) in state.options.columns.iter_mut().enumerate() {
                        ui.push_id(index, |ui| {
                            ui.horizontal(|ui| {
                                ui.set_min_height(22.0);
                                let label_width = (ui.available_width() - 90.0).max(80.0);
                                ui.allocate_ui_with_layout(
                                    egui::vec2(label_width, 22.0),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        ui.add_enabled_ui(
                                            column.field != Field::Column(ColumnId::Name),
                                            |ui| {
                                                ui.checkbox(&mut column.enabled, &column.label);
                                            },
                                        );
                                    },
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui
                                            .add_enabled(index + 1 < count, egui::Button::new("↓"))
                                            .on_hover_text(t!("export_listing.move_down"))
                                            .clicked()
                                        {
                                            movement = Some((index, index + 1));
                                        }
                                        if ui
                                            .add_enabled(index > 0, egui::Button::new("↑"))
                                            .on_hover_text(t!("export_listing.move_up"))
                                            .clicked()
                                        {
                                            movement = Some((index, index - 1));
                                        }
                                    },
                                );
                            });
                        });
                    }
                });
        });
    if let Some((a, b)) = movement {
        state.options.columns.swap(a, b);
    }
}
