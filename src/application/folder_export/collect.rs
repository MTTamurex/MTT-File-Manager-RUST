use super::*;
use crate::domain::file_entry::{get_file_type_string, FileEntry};
use crate::infrastructure::windows::{
    column_properties::{PropertyReader, PropertyValue},
    formatting,
};
use std::{fs, os::windows::fs::MetadataExt, path::Path};

struct Entry {
    file: FileEntry,
    descend: bool,
    cells: Vec<String>,
    sort_value: Option<PropertyValue>,
    tags: String,
    extension: String,
    size: Option<u64>,
}

fn tags(options: &Options, path: &Path) -> String {
    crate::domain::file_tag::tag_ids_for_path(&options.tags, path)
        .unwrap_or_default()
        .iter()
        .filter_map(|id| options.tag_names.get(id).cloned())
        .collect::<Vec<_>>()
        .join("; ")
}

fn read_entry(
    path: PathBuf,
    options: &Options,
    progress: &Progress,
    skipped: &mut usize,
) -> Result<Entry, ExportError> {
    let metadata = fs::symlink_metadata(&path).map_err(|_| ExportError::Read)?;
    // The directory attribute also identifies directory symlinks/junctions.
    // Do not reject all reparse points: cloud placeholders use them as well.
    let directory = metadata.file_attributes() & 0x10 != 0;
    let descend = directory && !metadata.file_type().is_symlink();
    let size = if directory {
        super::sizes::resolve(&path, descend, options, progress, skipped)?
    } else {
        Some(metadata.len())
    };
    let seconds = |time: std::io::Result<std::time::SystemTime>| {
        time.ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|time| time.as_secs())
    };
    let file = FileEntry {
        name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        path,
        is_dir: directory,
        size: if directory { 0 } else { metadata.len() },
        modified: seconds(metadata.modified()).unwrap_or(0),
        created: seconds(metadata.created()),
        folder_cover: None,
        drive_info: None,
        sync_status: Default::default(),
        is_hidden: metadata.file_attributes() & 2 != 0,
        recycle_bin: None,
    };
    let tag_text = tags(options, &file.path);
    let needs_properties = options.columns.iter().any(|column| {
        column.enabled && matches!(column.field, Field::Column(ColumnId::Property(_)))
    }) || matches!(options.sort, SortMode::Column(ColumnId::Property(_)));
    let reader = needs_properties
        .then(|| PropertyReader::open(&file.path).ok())
        .flatten();
    let sort_value = match options.sort {
        SortMode::Column(ColumnId::Property(id)) => {
            reader.as_ref().and_then(|reader| reader.read(id))
        }
        _ => None,
    };
    let cells = options
        .columns
        .iter()
        .filter(|column| column.enabled)
        .map(|column| match column.field {
            Field::RelativePath => file
                .path
                .strip_prefix(&options.root)
                .unwrap_or(&file.path)
                .to_string_lossy()
                .into_owned(),
            Field::Column(ColumnId::Name) => file.name.clone(),
            Field::Column(ColumnId::Date) => {
                formatting::format_date_with_format(file.modified, options.date_format)
            }
            Field::Column(ColumnId::Size) => size.map(|size| size.to_string()).unwrap_or_default(),
            Field::Column(ColumnId::Type) => get_file_type_string(&file),
            Field::Column(ColumnId::Tags) => tag_text.clone(),
            Field::Column(ColumnId::Status) => {
                use crate::domain::file_entry::SyncStatus;
                let key = match crate::infrastructure::onedrive::sync_status_for_path(&file.path)
                    .unwrap_or_default()
                {
                    SyncStatus::None => return String::new(),
                    SyncStatus::CloudOnly => "export_listing.cloud_only",
                    SyncStatus::Syncing => "export_listing.syncing",
                    SyncStatus::Pinned => "export_listing.pinned",
                    SyncStatus::LocallyAvailable => "export_listing.local",
                };
                rust_i18n::t!(key).to_string()
            }
            Field::Column(ColumnId::Property(id)) => reader
                .as_ref()
                .and_then(|reader| reader.read(id))
                .map(|value| {
                    value
                        .display_value(id)
                        .text_with_format(options.date_format)
                })
                .unwrap_or_default(),
            Field::Column(_) => String::new(),
        })
        .collect();
    let extension = file
        .name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default();
    Ok(Entry {
        file,
        descend,
        cells,
        sort_value,
        tags: tag_text,
        extension,
        size,
    })
}

fn sort(entries: &mut [Entry], options: &Options) {
    entries.sort_by(|a, b| {
        use std::cmp::Ordering;
        let folder_order = match options.folders {
            FoldersPosition::First => b.file.is_dir.cmp(&a.file.is_dir),
            FoldersPosition::Last => a.file.is_dir.cmp(&b.file.is_dir),
            FoldersPosition::Mixed => Ordering::Equal,
        };
        folder_order.then_with(|| {
            if matches!(
                options.sort,
                SortMode::Size | SortMode::Column(ColumnId::Size)
            ) {
                match (a.size, b.size) {
                    (None, Some(_)) => return Ordering::Greater,
                    (Some(_), None) => return Ordering::Less,
                    _ => {}
                }
            }
            // Missing custom properties stay last, independently of direction,
            // matching the file view's asynchronous column sorter.
            let missing = match options.sort {
                SortMode::Column(ColumnId::Property(_)) => {
                    a.sort_value.is_none().cmp(&b.sort_value.is_none())
                }
                SortMode::Column(ColumnId::Tags) => a.tags.is_empty().cmp(&b.tags.is_empty()),
                _ => Ordering::Equal,
            };
            if missing != Ordering::Equal {
                return missing;
            }
            let order = match options.sort {
                SortMode::Date | SortMode::Column(ColumnId::Date) => {
                    a.file.modified.cmp(&b.file.modified)
                }
                SortMode::Size | SortMode::Column(ColumnId::Size) => a.size.cmp(&b.size),
                SortMode::Type | SortMode::Column(ColumnId::Type) => a.extension.cmp(&b.extension),
                SortMode::Column(ColumnId::Property(_)) => match (&a.sort_value, &b.sort_value) {
                    (Some(a), Some(b)) => a.compare(b),
                    (None, Some(_)) => Ordering::Less,
                    (Some(_), None) => Ordering::Greater,
                    _ => Ordering::Equal,
                },
                SortMode::Column(ColumnId::Tags) => natord::compare_ignore_case(&a.tags, &b.tags),
                _ => natord::compare_ignore_case(&a.file.name, &b.file.name),
            };
            let custom = matches!(
                options.sort,
                SortMode::Column(ColumnId::Property(_) | ColumnId::Tags)
            );
            let names = || {
                natord::compare_ignore_case(&a.file.name, &b.file.name)
                    .then_with(|| a.file.path.cmp(&b.file.path))
            };
            if custom {
                (if options.descending {
                    order.reverse()
                } else {
                    order
                })
                .then_with(names)
            } else {
                let order = order.then_with(names);
                if options.descending {
                    order.reverse()
                } else {
                    order
                }
            }
        })
    });
}

fn children(
    path: &Path,
    skipped: &mut usize,
    progress: &Progress,
) -> Result<Vec<PathBuf>, ExportError> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(path).map_err(|_| ExportError::Read)? {
        progress.check()?;
        match entry {
            Ok(entry) => paths.push(entry.path()),
            Err(_) => *skipped += 1,
        }
    }
    Ok(paths)
}

fn read_batch(
    paths: Vec<PathBuf>,
    options: &Options,
    progress: &Progress,
    skipped: &mut usize,
) -> Result<std::vec::IntoIter<Entry>, ExportError> {
    let mut entries = Vec::new();
    for path in paths {
        progress.check()?;
        match read_entry(path, options, progress, skipped) {
            Ok(entry) => {
                entries.push(entry);
                progress.items.fetch_add(1, Ordering::Relaxed);
            }
            Err(ExportError::Cancelled) => return Err(ExportError::Cancelled),
            Err(_) => *skipped += 1,
        }
    }
    sort(&mut entries, options);
    Ok(entries.into_iter())
}

pub(super) fn collect(options: &Options, progress: &Progress) -> Result<Listing, ExportError> {
    progress.check()?;
    let mut skipped = 0;
    let paths = match &options.selection {
        Some(paths) => {
            // A selected ancestor already covers a descendant during recursive export.
            let mut paths = paths.clone();
            paths.sort();
            paths.dedup();
            let mut roots: Vec<PathBuf> = Vec::new();
            let mut covering: Option<PathBuf> = None;
            for path in paths {
                progress.check()?;
                if options.recursive && covering.as_ref().is_some_and(|root| path.starts_with(root))
                {
                    continue;
                }
                // A link is listed but not traversed, so it cannot replace an
                // explicitly selected descendant in the export's source set.
                covering = (options.recursive
                    && fs::symlink_metadata(&path).is_ok_and(|metadata| {
                        metadata.is_dir() && !metadata.file_type().is_symlink()
                    }))
                .then(|| path.clone());
                roots.push(path);
            }
            roots
        }
        None => children(&options.root, &mut skipped, progress)?,
    };
    let mut rows = Vec::new();
    // Explicit stack avoids overflowing the thread stack on deeply nested trees.
    let mut stack = vec![(read_batch(paths, options, progress, &mut skipped)?, 0)];
    while let Some((entries, depth)) = stack.last_mut() {
        progress.check()?;
        if let Some(entry) = entries.next() {
            let depth = *depth;
            let included = match options.contents {
                Contents::Both => true,
                Contents::Files => !entry.file.is_dir,
                Contents::Folders => entry.file.is_dir,
            };
            rows.push(Row {
                depth,
                directory: entry.file.is_dir,
                included,
                name: entry.file.name,
                cells: entry.cells,
                size: entry.size,
                traversable: entry.descend,
                complete: true,
            });
            if options.recursive && entry.descend {
                let index = rows.len() - 1;
                let before = skipped;
                match children(&entry.file.path, &mut skipped, progress) {
                    Ok(paths) => {
                        let entries = read_batch(paths, options, progress, &mut skipped)?;
                        rows[index].complete = before == skipped;
                        stack.push((entries, depth + 1));
                    }
                    Err(ExportError::Cancelled) => return Err(ExportError::Cancelled),
                    Err(_) => {
                        skipped += 1;
                        rows[index].complete = false;
                    }
                }
            }
        } else {
            stack.pop();
        }
    }
    super::sizes::finish(&mut rows, options, progress)?;
    Ok(Listing { rows, skipped })
}
