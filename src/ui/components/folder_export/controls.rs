use super::*;
use crate::ui::{components::settings_ui as kit, theme};

pub(super) fn render(ui: &mut egui::Ui, state: &mut FolderExportState) {
    ui.spacing_mut().item_spacing = egui::vec2(10.0, 6.0);
    if ui.available_width() >= 620.0 {
        ui.columns(2, |columns| {
            source(&mut columns[0], state);
            output(&mut columns[1], state);
        });
    } else {
        source(ui, state);
        ui.add_space(8.0);
        output(ui, state);
    }
    ui.add_space(4.0);
    ui.separator();
    egui::CollapsingHeader::new(egui::RichText::new(t!("export_listing.columns")).strong())
        .id_salt("export_columns_section")
        .default_open(false)
        .show(ui, |ui| super::columns::render(ui, state))
        .header_response
        .on_hover_text(t!("export_listing.columns_hint"));
    egui::CollapsingHeader::new(egui::RichText::new(t!("export_listing.sorting")).strong())
        .id_salt("export_sorting_section")
        .default_open(false)
        .show(ui, |ui| super::sorting::render(ui, state));
}

fn source(ui: &mut egui::Ui, state: &mut FolderExportState) {
    ui.strong(t!("export_listing.source_content"));
    ui.radio_value(&mut state.selected_only, false, t!("export_listing.all"));
    ui.add_enabled_ui(!state.selected_paths.is_empty(), |ui| {
        ui.radio_value(
            &mut state.selected_only,
            true,
            t!(
                "export_listing.selected",
                count = state.selected_paths.len()
            ),
        );
    });
    ui.add_space(4.0);
    ui.label(t!("export_listing.include"));
    egui::ComboBox::from_id_salt("export_contents")
        .width(ui.available_width())
        .selected_text(match state.options.contents {
            Contents::Both => t!("export_listing.both"),
            Contents::Files => t!("export_listing.files"),
            Contents::Folders => t!("export_listing.folders"),
        })
        .show_ui(ui, |ui| {
            ui.selectable_value(
                &mut state.options.contents,
                Contents::Both,
                t!("export_listing.both"),
            );
            ui.selectable_value(
                &mut state.options.contents,
                Contents::Files,
                t!("export_listing.files"),
            );
            ui.selectable_value(
                &mut state.options.contents,
                Contents::Folders,
                t!("export_listing.folders"),
            );
        });
    if ui
        .checkbox(&mut state.options.recursive, t!("export_listing.recursive"))
        .changed()
        && state.options.recursive
    {
        if let Some(column) = state
            .options
            .columns
            .iter_mut()
            .find(|column| column.field == Field::RelativePath)
        {
            column.enabled = true;
        }
    }
    ui.checkbox(
        &mut state.options.calculate_sizes,
        t!("export_listing.calculate_sizes"),
    );
    ui.label(
        egui::RichText::new(t!("export_listing.sizes_hint")).color(theme::text_color(ui.ctx())),
    );
    ui.label(
        egui::RichText::new(t!("export_listing.links_hint"))
            .color(theme::secondary_text_color(ui.ctx())),
    );
}

fn output(ui: &mut egui::Ui, state: &mut FolderExportState) {
    ui.strong(t!("export_listing.format_destination"));
    ui.label(t!("export_listing.format_label"));
    let labels = ["CSV", "TSV", "TXT", "HTML"];
    let formats = [Format::Csv, Format::Tsv, Format::Text, Format::Html];
    let selected = formats
        .iter()
        .position(|format| *format == state.options.format)
        .unwrap_or(0);
    if let Some(index) = kit::segmented_choice(ui, &labels, selected) {
        state.options.format = formats[index];
    }
    if state.options.format == Format::Csv {
        ui.label(t!("export_listing.separator"));
        egui::ComboBox::from_id_salt("export_separator")
            .width(ui.available_width())
            .selected_text(match state.options.csv_separator {
                CsvSeparator::Comma => t!("export_listing.comma"),
                CsvSeparator::Semicolon => t!("export_listing.semicolon"),
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut state.options.csv_separator,
                    CsvSeparator::Comma,
                    t!("export_listing.comma"),
                );
                ui.selectable_value(
                    &mut state.options.csv_separator,
                    CsvSeparator::Semicolon,
                    t!("export_listing.semicolon"),
                );
            });
    }
    ui.add_space(4.0);
    ui.label(t!("export_listing.destination"));
    ui.radio_value(&mut state.clipboard, false, t!("export_listing.file"));
    ui.radio_value(&mut state.clipboard, true, t!("export_listing.clipboard"));
    if state.clipboard && state.options.format == Format::Html {
        ui.label(t!("export_listing.html_clipboard"));
    }
    if state.options.format == Format::Text && state.options.recursive {
        ui.checkbox(&mut state.options.tree, t!("export_listing.tree"));
    }
}
