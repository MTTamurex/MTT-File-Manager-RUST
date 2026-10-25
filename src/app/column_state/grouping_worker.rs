//! Complete-list categorization, including Explorer's native category ordering, off the UI thread.
use crate::application::grouping::{
    columns::{alphabetic_category, date_category, size_category, ColumnGroups},
    GroupKey,
};
use crate::domain::{file_entry::FileEntry, file_tag::FileTag, list_columns::ColumnId};
use crate::infrastructure::windows::{
    column_properties::{
        FolderGrouping, NativeCategory, PropertyGrouping, PropertyReadError, PropertyReader,
        SchemaGroup,
    },
    com_scope::ComScope,
};
use eframe::egui;
use lru::LruCache;
mod ordering;
use ordering::{order_categories, CategoryOrder};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering as AtomicOrdering},
        Arc,
    },
};

pub(super) struct GroupRequest {
    pub signature: u64,
    pub items: Arc<Vec<FileEntry>>,
    pub column: ColumnId,
    pub tags: Arc<rustc_hash::FxHashMap<String, Vec<i64>>>,
    pub definitions: rustc_hash::FxHashMap<i64, FileTag>,
    pub cancelled: Arc<AtomicBool>,
    pub ctx: egui::Context,
}

fn column_key(column: ColumnId, identity: String, label: String) -> GroupKey {
    GroupKey::Column {
        column,
        identity,
        label,
    }
}

fn text_key(column: ColumnId, text: String) -> Option<(GroupKey, CategoryOrder)> {
    (!text.is_empty()).then(|| {
        (
            column_key(column, format!("text:{}", text.to_lowercase()), text),
            CategoryOrder::Other,
        )
    })
}

pub(super) fn run(request: &GroupRequest) -> Option<ColumnGroups> {
    let _com = if matches!(request.column, ColumnId::Property(_)) {
        let scope = ComScope::sta();
        if !scope.is_initialized() {
            return None;
        }
        Some(scope)
    } else {
        None
    };
    let mut folders =
        LruCache::<PathBuf, Option<FolderGrouping>>::new(std::num::NonZeroUsize::new(32).unwrap());
    let schema = match request.column {
        ColumnId::Property(id) => PropertyGrouping::new(id).ok(),
        _ => None,
    };
    let mut result = ColumnGroups::default();
    let mut categories = HashMap::new();
    let mut canonical = HashMap::new();
    for item in request.items.iter() {
        if request.cancelled.load(AtomicOrdering::Acquire) {
            return None;
        }
        let classified = match request.column {
            ColumnId::Property(id) => {
                if matches!(
                    PropertyReader::check_safe(&item.path),
                    Err(PropertyReadError::Deferred)
                ) {
                    result.deferred = true;
                    continue;
                }
                let parent = item.path.parent().unwrap_or(std::path::Path::new(""));
                if !folders.contains(parent) {
                    folders.put(parent.to_owned(), FolderGrouping::open(parent, id).ok());
                }
                let native = folders
                    .get(parent)
                    .and_then(|folder| folder.as_ref())
                    .and_then(|folder| folder.category(&item.path).ok());
                if let Some(NativeCategory {
                    id: category,
                    label,
                }) = native.filter(|category| !category.label.is_empty())
                {
                    Some((
                        column_key(
                            request.column,
                            format!("native:{}", label.to_lowercase()),
                            label,
                        ),
                        CategoryOrder::Native {
                            parent: parent.to_owned(),
                            id: category,
                        },
                    ))
                } else {
                    let value = match PropertyReader::open(&item.path) {
                        Ok(reader) => reader.read(id),
                        Err(PropertyReadError::Deferred) => {
                            result.deferred = true;
                            None
                        }
                        Err(PropertyReadError::Unavailable) => None,
                    };
                    value.and_then(|value| {
                        let group = schema.as_ref()?.classify(&value)?;
                        match group {
                            SchemaGroup::Range { index, label } => Some((
                                column_key(request.column, format!("range:{index}"), label),
                                CategoryOrder::Range(index),
                            )),
                            SchemaGroup::Date(timestamp) => {
                                Some((date_category(timestamp), CategoryOrder::Other))
                            }
                            SchemaGroup::Size(bytes) => {
                                Some((size_category(bytes), CategoryOrder::Other))
                            }
                            SchemaGroup::Alphabetic(text) => {
                                Some((alphabetic_category(&text), CategoryOrder::Other))
                            }
                            SchemaGroup::Text(text) => text_key(request.column, text)
                                .map(|(key, _)| (key, CategoryOrder::Typed(value))),
                        }
                    })
                }
            }
            ColumnId::Tags => {
                let names = crate::domain::file_tag::tag_ids_for_path(&request.tags, &item.path)
                    .into_iter()
                    .flatten()
                    .filter_map(|id| request.definitions.get(id))
                    .map(|tag| tag.name.as_str())
                    .collect::<Vec<_>>()
                    .join("; ");
                text_key(request.column, names)
            }
            ColumnId::OriginalLocation => item
                .recycle_original_path()
                .and_then(|path| path.parent())
                .and_then(|path| text_key(request.column, path.to_string_lossy().into_owned())),
            _ => None,
        };
        if let Some((key, order)) = classified {
            let key = if let GroupKey::Column { identity, .. } = &key {
                canonical.entry(identity.clone()).or_insert(key).clone()
            } else {
                key
            };
            categories.entry(key.clone()).or_insert(order);
            result.assignments.insert(item.path.clone(), key);
        }
    }
    if request.cancelled.load(AtomicOrdering::Acquire) {
        return None;
    }
    result.order = order_categories(categories, &folders);
    (!request.cancelled.load(AtomicOrdering::Acquire)).then_some(result)
}

#[cfg(test)]
#[path = "grouping_worker_tests.rs"]
mod tests;
