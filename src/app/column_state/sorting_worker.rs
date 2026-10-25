//! Native property reads and typed comparisons stay on the sorting worker's COM thread.
use crate::domain::file_entry::{FileEntry, FoldersPosition};
use crate::domain::list_columns::ColumnId;
use crate::infrastructure::windows::column_properties::{
    PropertyReadError, PropertyReader, PropertyValue,
};
use eframe::egui;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

pub(super) struct SortRequest {
    pub signature: u64,
    pub items: Arc<Vec<FileEntry>>,
    pub column: ColumnId,
    pub descending: bool,
    pub folders: FoldersPosition,
    pub tags: Arc<rustc_hash::FxHashMap<String, Vec<i64>>>,
    pub definitions: rustc_hash::FxHashMap<i64, crate::domain::file_tag::FileTag>,
    pub cancelled: Arc<AtomicBool>,
    pub ctx: egui::Context,
}

#[derive(Default)]
pub(super) struct SortResult {
    pub ranks: HashMap<PathBuf, usize>,
    pub deferred: bool,
}

pub(super) enum Value {
    Property(PropertyValue),
    Text(String),
    Missing,
}
impl Value {
    pub(super) fn compare(&self, other: &Self, descending: bool) -> std::cmp::Ordering {
        use std::cmp::Ordering::*;
        let result = match (self, other) {
            (Self::Missing, Self::Missing) => Equal,
            (Self::Missing, _) => return Greater,
            (_, Self::Missing) => return Less,
            (Self::Property(a), Self::Property(b)) => a.compare(b),
            (Self::Text(a), Self::Text(b)) => natord::compare_ignore_case(a, b),
            _ => Equal,
        };
        if descending {
            result.reverse()
        } else {
            result
        }
    }
}

pub(super) fn run(request: &SortRequest) -> Option<SortResult> {
    let _com = if matches!(request.column, ColumnId::Property(_)) {
        Some(crate::infrastructure::windows::metadata::utils::ComGuard::new()?)
    } else {
        None
    };
    let mut deferred = false;
    let mut values = Vec::with_capacity(request.items.len());
    for item in request.items.iter() {
        if request.cancelled.load(Ordering::Relaxed) {
            return None;
        }
        let value = match request.column {
            ColumnId::Property(id) => match PropertyReader::open(&item.path) {
                Ok(reader) => reader
                    .read(id)
                    .map(Value::Property)
                    .unwrap_or(Value::Missing),
                Err(PropertyReadError::Deferred) => {
                    deferred = true;
                    Value::Missing
                }
                Err(PropertyReadError::Unavailable) => Value::Missing,
            },
            ColumnId::OriginalLocation => item
                .recycle_original_path()
                .and_then(|path| path.parent())
                .map(|path| Value::Text(path.to_string_lossy().into_owned()))
                .unwrap_or(Value::Missing),
            ColumnId::Tags => {
                let names = crate::domain::file_tag::tag_ids_for_path(&request.tags, &item.path)
                    .into_iter()
                    .flatten()
                    .filter_map(|id| request.definitions.get(id))
                    .map(|tag| tag.name.as_str())
                    .collect::<Vec<_>>()
                    .join("; ");
                if names.is_empty() {
                    Value::Missing
                } else {
                    Value::Text(names)
                }
            }
            _ => Value::Missing,
        };
        values.push(value);
    }
    let mut indices: Vec<usize> = (0..request.items.len()).collect();
    indices.sort_by(|&a, &b| {
        let (a_item, b_item) = (&request.items[a], &request.items[b]);
        let a_dir = a_item.is_dir && !a_item.is_archive();
        let b_dir = b_item.is_dir && !b_item.is_archive();
        if a_dir != b_dir && request.folders != FoldersPosition::Mixed {
            return if request.folders == FoldersPosition::First {
                b_dir.cmp(&a_dir)
            } else {
                a_dir.cmp(&b_dir)
            };
        }
        values[a]
            .compare(&values[b], request.descending)
            .then_with(|| natord::compare_ignore_case(&a_item.name, &b_item.name))
            .then_with(|| a_item.path.cmp(&b_item.path))
    });
    if request.cancelled.load(Ordering::Relaxed) {
        return None;
    }
    Some(SortResult {
        ranks: indices
            .into_iter()
            .enumerate()
            .map(|(rank, index)| (request.items[index].path.clone(), rank))
            .collect(),
        deferred,
    })
}
