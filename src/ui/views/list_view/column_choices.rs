//! Scope-aware catalog aliases and searchable choices, independent of the picker UI.
use crate::domain::list_columns::{ColumnId, ColumnLayout, ColumnScope};
use crate::infrastructure::windows::column_properties::PropertyColumn;
use std::collections::HashSet;

fn scoped_id(column: &PropertyColumn, scope: ColumnScope) -> ColumnId {
    match (column.canonical_name.as_str(), scope) {
        ("System.DateModified", ColumnScope::Files)
        | ("System.Recycle.DateDeleted", ColumnScope::RecycleBin) => ColumnId::Date,
        _ => column.id,
    }
}

fn available(column: &PropertyColumn, scope: ColumnScope) -> bool {
    if scope != ColumnScope::RecycleBin && column.canonical_name.starts_with("System.Recycle.") {
        return false;
    }
    if scope != ColumnScope::Computer
        && matches!(column.id, ColumnId::FreeSpace | ColumnId::TotalSpace)
    {
        return false;
    }
    scope != ColumnScope::Computer
        || ![
            "System.Photo.",
            "System.Music.",
            "System.Audio.",
            "System.Video.",
        ]
        .iter()
        .any(|prefix| column.canonical_name.starts_with(prefix))
}

pub(super) fn choices(
    layout: &ColumnLayout,
    catalog: &[PropertyColumn],
    scope: ColumnScope,
    label: impl Fn(ColumnId) -> String,
) -> Vec<(ColumnId, String, String)> {
    let mut result: Vec<_> = layout
        .columns
        .iter()
        .map(|setting| {
            let canonical = catalog
                .iter()
                .find(|column| scoped_id(column, scope) == setting.id)
                .map(|column| column.canonical_name.clone())
                .unwrap_or_default();
            (setting.id, label(setting.id), canonical)
        })
        .collect();
    let mut seen: HashSet<_> = result.iter().map(|(id, _, _)| *id).collect();
    for id in [ColumnId::Tags, ColumnId::OriginalLocation] {
        if (id != ColumnId::OriginalLocation || scope == ColumnScope::RecycleBin) && seen.insert(id)
        {
            let canonical = catalog
                .iter()
                .find(|column| scoped_id(column, scope) == id)
                .map(|column| column.canonical_name.clone())
                .unwrap_or_default();
            result.push((id, label(id), canonical));
        }
    }
    for column in catalog {
        let id = scoped_id(column, scope);
        if available(column, scope) && seen.insert(id) {
            result.push((id, column.label.clone(), column.canonical_name.clone()));
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::list_columns::PropertyId;

    fn property(pid: u32, canonical: &str) -> PropertyColumn {
        PropertyColumn {
            id: ColumnId::Property(PropertyId { fmtid: 0, pid }),
            canonical_name: canonical.into(),
            label: "Property".into(),
        }
    }

    #[test]
    fn selecting_a_property_does_not_remove_it_from_canonical_search() {
        let column = property(1, "System.DateCreated");
        let mut layout = ColumnLayout::defaults(ColumnScope::Files);
        let before = choices(
            &layout,
            std::slice::from_ref(&column),
            ColumnScope::Files,
            |_| "Label".into(),
        );
        layout.set_visible(column.id, true);
        let after = choices(
            &layout,
            std::slice::from_ref(&column),
            ColumnScope::Files,
            |_| "Label".into(),
        );
        for choices in [before, after] {
            assert_eq!(
                choices
                    .iter()
                    .filter(|(_, _, name)| name == "System.DateCreated")
                    .count(),
                1
            );
        }
    }

    #[test]
    fn default_date_alias_is_searchable_and_remains_scope_specific() {
        let catalog = vec![
            property(1, "System.DateModified"),
            property(2, "System.Recycle.DateDeleted"),
        ];
        for (scope, canonical) in [
            (ColumnScope::Files, "System.DateModified"),
            (ColumnScope::RecycleBin, "System.Recycle.DateDeleted"),
        ] {
            let result = choices(&ColumnLayout::defaults(scope), &catalog, scope, |_| {
                "Date".into()
            });
            assert_eq!(
                result
                    .iter()
                    .find(|(id, _, _)| *id == ColumnId::Date)
                    .unwrap()
                    .2,
                canonical
            );
            assert_eq!(
                result
                    .iter()
                    .filter(|(_, _, name)| name == canonical)
                    .count(),
                1
            );
        }
    }
}
