use super::*;
use crate::domain::{
    file_entry::{FoldersPosition, SortMode},
    list_columns::ColumnId,
};

pub(super) fn render(ui: &mut egui::Ui, state: &mut FolderExportState) {
    let label = match state.options.sort {
        SortMode::Name => t!("export_listing.name").to_string(),
        SortMode::Date => t!("export_listing.modified").to_string(),
        SortMode::Size => t!("export_listing.size").to_string(),
        SortMode::Type => t!("export_listing.type").to_string(),
        SortMode::Column(id) => state
            .options
            .columns
            .iter()
            .find(|column| column.field == Field::Column(id))
            .map(|column| column.label.clone())
            .unwrap_or_default(),
        _ => t!("export_listing.name").to_string(),
    };
    ui.horizontal_wrapped(|ui| {
        ui.label(t!("export_listing.sort_by"));
        egui::ComboBox::from_id_salt("export_sort")
            .width(160.0)
            .selected_text(label)
            .show_ui(ui, |ui| {
                for (mode, label) in [
                    (SortMode::Name, t!("export_listing.name")),
                    (SortMode::Date, t!("export_listing.modified")),
                    (SortMode::Size, t!("export_listing.size")),
                    (SortMode::Type, t!("export_listing.type")),
                ] {
                    ui.selectable_value(&mut state.options.sort, mode, label);
                }
                for column in &state.options.columns {
                    if let Field::Column(id @ (ColumnId::Property(_) | ColumnId::Tags)) =
                        column.field
                    {
                        ui.selectable_value(
                            &mut state.options.sort,
                            SortMode::Column(id),
                            &column.label,
                        );
                    }
                }
            });
        ui.checkbox(
            &mut state.options.descending,
            t!("export_listing.descending"),
        );
    });
    let labels = [
        t!("export_listing.folders_first"),
        t!("export_listing.folders_last"),
        t!("export_listing.mixed"),
    ];
    let positions = [
        FoldersPosition::First,
        FoldersPosition::Last,
        FoldersPosition::Mixed,
    ];
    let selected = positions
        .iter()
        .position(|position| *position == state.options.folders)
        .unwrap_or(0);
    egui::ComboBox::from_id_salt("export_folder_position")
        .width(180.0)
        .selected_text(labels[selected].as_ref())
        .show_ui(ui, |ui| {
            for (index, position) in positions.into_iter().enumerate() {
                ui.selectable_value(&mut state.options.folders, position, labels[index].as_ref());
            }
        });
    ui.label(
        egui::RichText::new(t!("export_listing.sort_hint"))
            .color(crate::ui::theme::secondary_text_color(ui.ctx())),
    );
}
