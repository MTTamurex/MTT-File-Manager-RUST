//! Use native category order or typed property values, never numeric display captions.
use crate::application::grouping::{group_label, GroupKey};
use crate::infrastructure::windows::column_properties::{FolderGrouping, PropertyValue};
use lru::LruCache;
use std::{collections::HashMap, path::PathBuf};

pub(super) enum CategoryOrder {
    Native { parent: PathBuf, id: u32 },
    Range(u32),
    Typed(PropertyValue),
    Other,
}

pub(super) fn order_categories(
    categories: HashMap<GroupKey, CategoryOrder>,
    folders: &LruCache<PathBuf, Option<FolderGrouping>>,
) -> HashMap<GroupKey, usize> {
    let mut ordered: Vec<_> = categories
        .into_iter()
        .map(|(key, order)| {
            let label = group_label(&key);
            (key, order, label)
        })
        .collect();
    // Mixed folders can use different category providers. One strategy for the entire
    // set avoids contradictory comparisons between native, range and fallback groups.
    let native_parent = ordered.first().and_then(|(_, order, _)| match order {
        CategoryOrder::Native {parent, ..} if ordered.iter().all(|(_, order, _)| matches!(order, CategoryOrder::Native {parent: other, ..} if other == parent)) => Some(parent.clone()),
        _ => None,
    });
    let native = native_parent
        .as_ref()
        .and_then(|parent| folders.peek(parent))
        .and_then(|folder| folder.as_ref());
    let ranges_only = ordered
        .iter()
        .all(|(_, order, _)| matches!(order, CategoryOrder::Range(_)));
    let typed_only = ordered
        .iter()
        .all(|(_, order, _)| matches!(order, CategoryOrder::Typed(_)));
    let dates_only = ordered
        .iter()
        .all(|(key, _, _)| matches!(key, GroupKey::Date(_)));
    let names_only = ordered
        .iter()
        .all(|(key, _, _)| matches!(key, GroupKey::Name(_)));
    let sizes_only = ordered
        .iter()
        .all(|(key, _, _)| matches!(key, GroupKey::Size(_)));
    ordered.sort_by(|(a, a_order, a_label), (b, b_order, b_label)| {
        match (a_order, b_order) {
            (CategoryOrder::Range(a), CategoryOrder::Range(b)) if ranges_only => return a.cmp(b),
            (CategoryOrder::Typed(a), CategoryOrder::Typed(b)) if typed_only => {
                return a.compare(b)
            }
            (CategoryOrder::Native { id: a, .. }, CategoryOrder::Native { id: b, .. }) => {
                if let Some(order) = native.and_then(|folder| folder.compare(*a, *b)) {
                    return order;
                }
            }
            _ => {}
        }
        match (a, b) {
            (GroupKey::Date(a), GroupKey::Date(b)) if dates_only => a.cmp(b),
            (GroupKey::Name(a), GroupKey::Name(b)) if names_only => a.cmp(b),
            (GroupKey::Size(a), GroupKey::Size(b)) if sizes_only => a.cmp(b),
            _ => natord::compare_ignore_case(a_label, b_label),
        }
    });
    ordered
        .into_iter()
        .enumerate()
        .map(|(rank, (key, _, _))| (key, rank))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::list_columns::ColumnId;
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;

    fn group(id: &str, caption: &str) -> GroupKey {
        GroupKey::Column {
            column: ColumnId::Tags,
            identity: id.into(),
            label: caption.into(),
        }
    }

    #[test]
    fn numeric_groups_are_ordered_by_raw_values_even_when_units_and_thousands_separators_differ() {
        let smaller = group("smaller", "900 KB");
        let larger = group("larger", "1.5 MB");
        let categories = HashMap::from([
            (
                smaller.clone(),
                CategoryOrder::Typed(PropertyValue::from(PROPVARIANT::from(900_000_u64))),
            ),
            (
                larger.clone(),
                CategoryOrder::Typed(PropertyValue::from(PROPVARIANT::from(1_500_000_u64))),
            ),
        ]);
        let folders = LruCache::new(std::num::NonZeroUsize::new(1).unwrap());
        let ranks = order_categories(categories, &folders);
        assert!(ranks[&smaller] < ranks[&larger]);
    }

    #[test]
    fn mixed_sources_use_one_consistent_caption_order_instead_of_partial_range_ordering() {
        let first = group("a", "Alpha");
        let middle = group("b", "Beta");
        let last = group("c", "Gamma");
        let categories = HashMap::from([
            (first.clone(), CategoryOrder::Range(10)),
            (middle.clone(), CategoryOrder::Other),
            (last.clone(), CategoryOrder::Range(0)),
        ]);
        let ranks = order_categories(
            categories,
            &LruCache::new(std::num::NonZeroUsize::new(1).unwrap()),
        );
        assert_eq!([ranks[&first], ranks[&middle], ranks[&last]], [0, 1, 2]);
    }
}
