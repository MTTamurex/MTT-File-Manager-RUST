use crate::app::ImageViewerApp;
use crate::application::folder_export::{worker::Job, *};
use crate::domain::{
    file_entry::SortMode,
    list_columns::{ColumnId, ColumnScope},
};
use std::path::PathBuf;

pub(crate) struct FolderExportState {
    pub options: Options,
    pub selected_paths: Vec<PathBuf>,
    pub selected_only: bool,
    pub clipboard: bool,
    pub job: Option<Job>,
    pub status: Option<String>,
}

pub(crate) fn available(app: &ImageViewerApp) -> bool {
    !crate::domain::special_paths::is_virtual_path(&app.navigation_state.current_path)
        && !app.current_location_is_archive_namespace()
        && std::path::Path::new(&app.navigation_state.current_path).is_absolute()
}

fn column_label(app: &ImageViewerApp, field: Field) -> String {
    let key = match field {
        Field::RelativePath => "export_listing.relative_path",
        Field::Column(ColumnId::Name) => "export_listing.name",
        Field::Column(ColumnId::Date) => "export_listing.modified",
        Field::Column(ColumnId::Size) => "export_listing.size",
        Field::Column(ColumnId::Type) => "export_listing.type",
        Field::Column(ColumnId::Status) => "column_headers.status",
        Field::Column(ColumnId::Tags) => "column_headers.app_tags",
        Field::Column(id) => {
            return app
                .columns
                .catalog
                .iter()
                .find(|column| column.id == id)
                .map(|column| column.label.clone())
                .unwrap_or_else(|| rust_i18n::t!("column_headers.custom_sort").to_string())
        }
    };
    rust_i18n::t!(key).to_string()
}

impl FolderExportState {
    pub fn open(app: &ImageViewerApp) -> Self {
        let mut selected_paths: Vec<_> = app.multi_selection.iter().cloned().collect();
        if selected_paths.is_empty() {
            selected_paths.extend(app.selected_file.as_ref().map(|file| file.path.clone()));
        }
        selected_paths.sort();
        selected_paths.dedup();
        let mut columns: Vec<_> = app
            .columns
            .preferences
            .layout(ColumnScope::Files)
            .columns
            .iter()
            .filter(|column| {
                !matches!(
                    column.id,
                    ColumnId::TotalSpace | ColumnId::FreeSpace | ColumnId::OriginalLocation
                )
            })
            .map(|column| ExportColumn {
                field: Field::Column(column.id),
                label: column_label(app, Field::Column(column.id)),
                enabled: column.visible,
            })
            .collect();
        for field in [
            Field::Column(ColumnId::Name),
            Field::Column(ColumnId::Date),
            Field::Column(ColumnId::Type),
            Field::Column(ColumnId::Size),
            Field::Column(ColumnId::Tags),
            Field::RelativePath,
        ] {
            if !columns.iter().any(|column| column.field == field) {
                columns.push(ExportColumn {
                    field,
                    label: column_label(app, field),
                    enabled: field == Field::Column(ColumnId::Name),
                });
            }
        }
        let sort = match app.sort_mode {
            SortMode::DriveTotalSpace | SortMode::DriveFreeSpace | SortMode::DriveLetter => {
                SortMode::Name
            }
            sort => sort,
        };
        Self {
            options: Options {
                root: PathBuf::from(&app.navigation_state.current_path),
                selection: None,
                recursive: false,
                contents: Contents::Both,
                format: Format::Csv,
                csv_separator: CsvSeparator::regional_default(),
                calculate_sizes: false,
                folder_sizes: app.folder_size_state.export_size_snapshot(),
                tree: true,
                columns,
                sort,
                descending: app.sort_descending,
                folders: app.folders_position,
                date_format: crate::domain::date_format::date_format(),
                tags: app.tag_assignments_normalized.clone(),
                tag_names: app
                    .tag_definitions
                    .iter()
                    .map(|(id, tag)| (*id, tag.name.clone()))
                    .collect(),
            },
            selected_paths,
            selected_only: false,
            clipboard: false,
            job: None,
            status: None,
        }
    }
}
